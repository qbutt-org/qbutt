const root = new URL("./", document.currentScript.src);
const key = "qbutt-org-language";
const requested = new URLSearchParams(location.search).get("lang");
const explicit = requested === "ru" || requested === "en" ? requested : null;
const preferred = explicit ?? localStorage.getItem(key)
  ?? (navigator.language.startsWith("ru") ? "ru" : "en");

if (explicit) localStorage.setItem(key, explicit);
if (preferred !== document.documentElement.lang) {
  location.replace(new URL(preferred === "en" ? "en/?lang=en" : "?lang=ru", root));
}
