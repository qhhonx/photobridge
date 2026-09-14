// Only replace media when the language changes, not when release data refreshes.
export function updateDemoLanguage(video, language) {
  if (!video) return;
  const locale = language === "zh" ? "zh" : "en";
  const source = `/media/demo-${locale}-v1.mp4`;
  if (video.getAttribute("src") === source) return;
  video.pause();
  video.poster = `/media/demo-${locale}-v1.jpg`;
  video.setAttribute("aria-label", locale === "zh"
    ? "PhotoBridge：iPhone 通过 Wi-Fi 备份到 Pixel，42 秒示例演示"
    : "PhotoBridge: iPhone to Pixel over Wi-Fi, a 42-second illustrated demo");
  video.src = source;
  // A language change starts the new version paused. Never autoplay audio.
  video.load();
}
