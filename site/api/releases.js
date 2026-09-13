import { latestRelease } from '../lib/releases.js';
export default async function handler(req,res) {
  try { const {feed,...release}=await latestRelease(); res.setHeader('Cache-Control','public, s-maxage=300, stale-while-revalidate=60'); res.status(200).json(release); }
  catch { res.setHeader('Cache-Control','no-store'); res.status(503).json({error:'release_unavailable'}); }
}
