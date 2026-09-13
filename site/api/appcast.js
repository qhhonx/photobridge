import { latestRelease } from '../lib/releases.js';
export default async function handler(req,res) {
  try {
    const release=await latestRelease();
    const response=await fetch(release.feed,{signal:AbortSignal.timeout(10000)});
    if(!response.ok) throw Error();
    const bytes=Buffer.from(await response.arrayBuffer());
    if(bytes.length>1048576 || !bytes.includes(Buffer.from('<rss'))) throw Error();
    // Preserve every byte: Sparkle verifies the feed signature, including whitespace.
    res.setHeader('Content-Type','application/xml'); res.setHeader('Cache-Control','public, s-maxage=300, stale-while-revalidate=60'); res.status(200).send(bytes);
  } catch { res.setHeader('Cache-Control','no-store'); res.status(503).send('Release feed unavailable'); }
}
