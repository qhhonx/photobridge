export function completeRelease(releases) {
  if (!Array.isArray(releases)) return null;
  const versions = release => { const match = /^v(\d+)\.(\d+)\.(\d+)(?:-beta\.(\d+))?$/.exec(release.tag_name); return match ? match.slice(1).map((v,i)=> v === undefined && i === 3 ? Number.MAX_SAFE_INTEGER : Number(v)) : null; };
  return releases.filter(r => !r.draft && r.published_at && versions(r)).sort((a,b) => {
    const x=versions(a), y=versions(b); for(let i=0;i<4;i++) if(x[i]!==y[i]) return y[i]-x[i]; return 0;
  }).map(r => {
    const version=r.tag_name.slice(1), prefix=`https://github.com/qhhonx/photobridge/releases/download/${r.tag_name}/`;
    const asset=name=>r.assets?.find(a=>a.name===name && a.size>0 && a.browser_download_url===prefix+name)?.browser_download_url;
    const mac=asset(`PhotoBridge-${version}-arm64.zip`),android=asset(`PhotoBridge-${version}-arm64.apk`),feed=asset('appcast.xml'),manifest=asset('android-update.json'),hashes=asset('SHA256SUMS');
    return mac && android && feed && manifest && hashes ? {version,mac,android,feed} : null;
  }).find(Boolean) || null;
}
export async function latestRelease() {
  const response=await fetch('https://api.github.com/repos/qhhonx/photobridge/releases?per_page=30',{headers:{Accept:'application/vnd.github+json','User-Agent':'PhotoBridge'},signal:AbortSignal.timeout(10000)});
  if(!response.ok) throw Error('Release service unavailable');
  const release=completeRelease(await response.json());
  if(!release) throw Error('No complete release');
  return release;
}
