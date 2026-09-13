//! Stream a file-backed asset envelope into the existing resumable store.
//! No request-wide buffering or implicit gallery/cloud completion.
use super::*;
use axum::{
    body::{Body, BodyDataStream},
    http::HeaderMap,
};
use futures_util::StreamExt;

struct Reader {
    stream: BodyDataStream,
    buffered: Bytes,
}
impl Reader {
    async fn next(&mut self) -> Result<bool> {
        let next = tokio::time::timeout(Duration::from_secs(30), self.stream.next())
            .await
            .map_err(|_| Error::Transport("bundle body stalled".into()))?;
        match next {
            None => Ok(false),
            Some(Err(_)) => Err(Error::Transport("bundle body interrupted".into())),
            Some(Ok(data)) => {
                // Real network frames are small; reject unreasonable custom frames.
                if data.len() > MAX_CHUNK_BYTES {
                    return Err(Error::Invalid("bundle frame size".into()));
                }
                self.buffered = data;
                Ok(true)
            }
        }
    }
    async fn exact(&mut self, length: usize) -> Result<Vec<u8>> {
        if length > MAX_CHUNK_BYTES {
            return Err(Error::Invalid("bundle read size".into()));
        }
        let mut result = Vec::with_capacity(length);
        while result.len() < length {
            if self.buffered.is_empty() && !self.next().await? {
                return Err(Error::Invalid("incomplete bundle".into()));
            }
            let take = (length - result.len()).min(self.buffered.len());
            result.extend_from_slice(&self.buffered.split_to(take));
        }
        Ok(result)
    }
    async fn end(&mut self) -> Result<()> {
        loop {
            if !self.buffered.is_empty() {
                return Err(Error::Invalid("trailing bundle bytes".into()));
            }
            if !self.next().await? {
                return Ok(());
            }
        }
    }
}
pub(super) async fn receive(
    axum::Extension(state): axum::Extension<ServerState>,
    headers: HeaderMap,
    body: Body,
) -> std::result::Result<Json<AssetStatus>, ApiError> {
    if headers.get("content-type").and_then(|v| v.to_str().ok()) != Some(BUNDLE_CONTENT_TYPE) {
        return Err(Error::Invalid("bundle content type".into()).into());
    }
    let mut reader = Reader {
        stream: body.into_data_stream(),
        buffered: Bytes::new(),
    };
    if reader.exact(8).await? != BUNDLE_MAGIC {
        return Err(Error::Invalid("bundle magic".into()).into());
    }
    let length = u32::from_be_bytes(
        reader
            .exact(4)
            .await?
            .try_into()
            .map_err(|_| Error::Invalid("bundle header".into()))?,
    ) as usize;
    if length == 0 || length > MAX_MANIFEST_BYTES {
        return Err(Error::Invalid("bundle manifest limit".into()).into());
    }
    let asset: Asset = serde_json::from_slice(&reader.exact(length).await?).map_err(Error::from)?;
    asset.validate()?;
    let expected = bundle_size(&asset)?;
    if let Some(length) = headers.get("content-length") {
        if length.to_str().ok().and_then(|s| s.parse::<u64>().ok()) != Some(expected) {
            return Err(Error::Invalid("bundle length".into()).into());
        }
    }
    let registration = asset.clone();
    let sender = state.sender_id.clone();
    let Json(status) = with_receiver(state.clone(), move |r| {
        let status = r.register(registration)?;
        if let Some(id) = sender {
            r.attribute_sender(&status.asset_id, &id)?;
        }
        Ok(status)
    })
    .await?;
    // A retained receipt is sufficient even when an original was deliberately
    // reclaimed. Do not attempt to recreate reclaimed files on a lost-response replay.
    if status.receipt == ReceiptState::Received {
        // Consume the declared envelope before replying. Closing an HTTP/1 request
        // while the client is still uploading can reset the connection instead of
        // delivering the retained receipt (notably on Windows). Keep reads bounded
        // and do not recreate or modify any stored resource.
        for resource in &asset.resources {
            let mut remaining = resource.size;
            while remaining > 0 {
                let count = remaining.min(MAX_CHUNK_BYTES as u64) as usize;
                reader.exact(count).await?;
                remaining -= count as u64;
            }
        }
        reader.end().await?;
        return Ok(Json(status));
    }
    let id = status.asset_id;
    for resource in asset.resources {
        let mut offset = 0;
        while offset < resource.size {
            let bytes = reader
                .exact((resource.size - offset).min(MAX_CHUNK_BYTES as u64) as usize)
                .await?;
            let count = bytes.len() as u64;
            let resource_hash = resource.sha256.clone();
            let asset_id = id.clone();
            let _ = with_receiver(state.clone(), move |receiver| {
                let snapshot = receiver.status(&asset_id)?;
                let existing = snapshot
                    .resources
                    .iter()
                    .find(|r| r.sha256 == resource_hash)
                    .ok_or(Error::Integrity)?;
                // Previous protocol clients may have stopped at a different chunk
                // boundary. Verify the replayed prefix separately before appending.
                let prefix = existing
                    .offset
                    .saturating_sub(offset)
                    .min(bytes.len() as u64) as usize;
                if prefix > 0 {
                    receiver.append(
                        &asset_id,
                        &resource_hash,
                        offset,
                        &bytes[..prefix],
                        &digest(&bytes[..prefix]),
                    )?;
                }
                if prefix < bytes.len() {
                    receiver.append(
                        &asset_id,
                        &resource_hash,
                        offset + prefix as u64,
                        &bytes[prefix..],
                        &digest(&bytes[prefix..]),
                    )?;
                }
                Ok(())
            })
            .await?;
            offset += count;
        }
    }
    reader.end().await?;
    with_receiver(state, move |r| r.commit(&id)).await
}
