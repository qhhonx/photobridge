const elements = [...document.querySelectorAll("[data-i18n]")];
const english = Object.fromEntries(
  elements.map((el) => [el.dataset.i18n, el.innerHTML]),
);
let language;
try {
  language = localStorage.getItem("language");
} catch {}
language =
  language === "en" || language === "zh"
    ? language
    : navigator.language.startsWith("zh")
      ? "zh"
      : "en";
let chinese = {},
  release = null,
  releaseFailed = false;
function render() {
  document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
  const copy = language === "zh" ? chinese : english;
  for (const el of elements)
    el.innerHTML = copy[el.dataset.i18n] || english[el.dataset.i18n];
  document.querySelector("#language").textContent =
    language === "zh" ? "English" : "中文";
  document.title =
    language === "zh"
      ? "PhotoBridge — 把照片备份到自己的设备"
      : "PhotoBridge — Your photos. Beyond one device.";
  const status = document.querySelector("#release-status");
  if (release) {
    status.textContent = `${release.version} · ${language === "zh" ? "最新发布版本" : "Latest release"}`;
    for (const platform of ["mac", "android"]) {
      const link = document.querySelector(`#download-${platform}`);
      link.href = release[platform];
      link.textContent =
        language === "zh"
          ? `下载 ${platform === "mac" ? "Mac 版" : "APK"}`
          : `Download ${platform === "mac" ? "for Mac" : "APK"}`;
    }
  } else if (releaseFailed)
    status.textContent =
      language === "zh"
        ? "暂时无法获取下载信息，请前往 GitHub 查看发布版本。"
        : "Downloads are temporarily unavailable. Check releases on GitHub.";
}
try {
  chinese = await fetch("/zh.json").then((r) => {
    if (!r.ok) throw Error();
    return r.json();
  });
} catch {
  language = "en";
}
render();
document.querySelector("#language").addEventListener("click", () => {
  language = language === "en" ? "zh" : "en";
  try {
    localStorage.setItem("language", language);
  } catch {}
  render();
});
try {
  const response = await fetch("/api/releases");
  if (!response.ok) throw Error();
  release = await response.json();
  if (
    !release ||
    !["mac", "android"].every(
      (key) =>
        typeof release[key] === "string" &&
        release[key].startsWith(
          "https://github.com/qhhonx/photobridge/releases/download/",
        ),
    )
  )
    throw Error();
} catch {
  release = null;
  releaseFailed = true;
}
render();
