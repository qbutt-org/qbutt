const userAgent = navigator.userAgent;
let platform;

if (!/Android|iPhone|iPad|Mobile|CrOS|ARM|aarch64/i.test(userAgent)) {
  if (/Windows NT/.test(userAgent) && /Win64|WOW64|x64|amd64/i.test(userAgent)) {
    platform = "windows-x64";
  } else if (/Linux/.test(userAgent) && /x86_64|x64|amd64/i.test(userAgent)) {
    platform = "linux-x64";
  }
}

const edition = document.querySelector(
  `.editions a[data-platform="${platform}"]`
);

if (edition) {
  const primary = document.createElement("a");
  primary.className = "download";
  primary.href = edition.href;
  primary.textContent = edition.dataset.downloadLabel;

  const editions = edition.closest(".editions");
  editions.before(primary);

  const summary = editions.querySelector("summary");
  summary.classList.remove("download");
  summary.textContent = summary.dataset.allLabel;
}
