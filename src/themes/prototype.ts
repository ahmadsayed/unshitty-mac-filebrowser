// THROWAWAY: three theme directions on the real mock app, ?variant=retro|cyberpunk|omarchy.
// No saved theme preferences, installed-app changes or production activation.
import "./prototype.css";

if (import.meta.env.DEV && new URLSearchParams(location.search).has("variant")) {
  const variants = [
    { id: "retro", name: "Retro / 1995", note: "Beveled chrome · navy title bar · pixel-era desktop" },
    { id: "cyberpunk", name: "Cyberpunk / Night Drive", note: "Neon cyan · magenta signals · midnight workstation" },
    { id: "omarchy", name: "Omarchy-inspired / Amber", note: "Warm charcoal · amber borders · quiet terminal typography" },
  ];
  let current = Math.max(0, variants.findIndex((v) => v.id === new URLSearchParams(location.search).get("variant")));
  const bar = document.createElement("nav");
  bar.className = "prototype-switcher";
  bar.setAttribute("aria-label", "Theme mock switcher");
  bar.innerHTML = `<span class="prototype-tag">DESIGN MOCK</span><button aria-label="Previous mock">←</button><div><strong></strong><small></small></div><button aria-label="Next mock">→</button>`;
  const apply = () => {
    const v = variants[current];
    document.documentElement.dataset.prototype = v.id;
    bar.querySelector("strong")!.textContent = v.name;
    bar.querySelector("small")!.textContent = v.note;
    const url = new URL(location.href);
    url.searchParams.set("variant", v.id);
    history.replaceState(null, "", url);
  };
  bar.querySelectorAll("button").forEach((button, i) => button.addEventListener("click", () => {
    current = (current + (i ? 1 : -1) + variants.length) % variants.length;
    apply();
  }));
  document.addEventListener("keydown", (event) => {
    if ([event.altKey, event.metaKey, event.ctrlKey, event.shiftKey].some(Boolean)) return;
    if ((event.target as HTMLElement).closest("input, textarea, [contenteditable], .dialog-layer")) return;
    const direction = new Map([["ArrowLeft", -1], ["ArrowRight", 1]]).get(event.key);
    if (!direction) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    current = (current + direction + variants.length) % variants.length;
    apply();
  }, true);
  apply();
  window.addEventListener("DOMContentLoaded", () => {
    document.body.append(bar);
    // Mock-only editorial chrome. The real virtualized view and navigation stay intact.
    const strip = document.createElement("div");
    strip.className = "prototype-section";
    strip.innerHTML = `<span class="prototype-section-label"></span><span class="prototype-section-detail">LOCAL FILESYSTEM <b>●</b> READ / WRITE</span>`;
    document.querySelector(".view-overlay")!.prepend(strip);
    const label = strip.querySelector(".prototype-section-label")!;
    const sync = () => {
      const title = document.title || "Home";
      label.textContent = document.documentElement.dataset.prototype === "retro" ? `File  Edit  View  Tools  Help` : `${title.toUpperCase()} / INDEX`;
    };
    new MutationObserver(sync).observe(document.documentElement, { attributes: true, attributeFilter: ["data-prototype"] });
    new MutationObserver(sync).observe(document.querySelector("title")!, { childList: true });
    sync();
  });
}
