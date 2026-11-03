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
  const editions = edition.closest(".editions");
  if (platform === "windows-x64") {
    const primary = document.createElement("a");
    primary.className = "download";
    primary.href = edition.href;
    primary.textContent = edition.dataset.downloadLabel;
    editions.parentElement.prepend(primary);
  }

  const summary = editions.querySelector("summary");
  summary.classList.remove("download");
  summary.textContent = summary.dataset.allLabel;
}

const command = document.getElementById("install-command");
const copy = document.querySelector(".install-copy");
const error = document.querySelector(".install-error");
const copyLabel = copy.getAttribute("aria-label");
let resetCopy;

copy.addEventListener("click", async () => {
  clearTimeout(resetCopy);
  error.hidden = true;
  copy.removeAttribute("data-copied");
  copy.setAttribute("aria-label", copyLabel);

  try {
    await navigator.clipboard.writeText(command.textContent);
    copy.setAttribute("data-copied", "");
    copy.setAttribute("aria-label", copy.dataset.copiedLabel);
    resetCopy = setTimeout(() => {
      copy.removeAttribute("data-copied");
      copy.setAttribute("aria-label", copyLabel);
    }, 1800);
  } catch {
    const range = document.createRange();
    range.selectNodeContents(command);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    error.hidden = false;
  }
});
