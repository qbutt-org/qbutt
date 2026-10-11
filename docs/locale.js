const root = new URL("./", document.currentScript.src);
const key = "qbutt-org-language";
const requested = new URLSearchParams(location.search).get("lang");
const explicit = requested === "ru" || requested === "en" ? requested : null;
const current = document.documentElement.lang;
const preferred = explicit ?? (current === "en" ? "en" : localStorage.getItem(key))
  ?? (navigator.language.startsWith("ru") ? "ru" : "en");

if (explicit) localStorage.setItem(key, explicit);
if (preferred !== current) {
  location.replace(new URL(preferred === "en" ? "en/" : "?lang=ru", root));
}

document.addEventListener("click", (event) => {
  const link = event.target.closest("a[data-locale]");
  if (link) localStorage.setItem(key, link.dataset.locale);
});
