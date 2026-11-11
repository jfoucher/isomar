// Isomar's website: palette switch, platform tabs, lightbox, copy buttons, and the visitor's OS first.
(() => {
  const root = document.documentElement;

  // ---- Palette (Day / Night), remembered per visitor ----
  const dark = matchMedia("(prefers-color-scheme: dark)");
  const current = () => root.dataset.theme || (dark.matches ? "night" : "day");
  const showPalette = () => {
    document.querySelectorAll("[data-theme-pick]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.themePick === current())));
  };
  document.querySelectorAll("[data-theme-pick]").forEach((b) =>
    b.addEventListener("click", () => {
      root.dataset.theme = b.dataset.themePick;
      try { localStorage.setItem("isomar-palette", b.dataset.themePick); } catch (e) {}
      showPalette();
    }),
  );
  dark.addEventListener("change", showPalette);
  showPalette();

  // ---- The visitor's platform first ----
  const ua = navigator.userAgent;
  // An iPad asks for the desktop site and says "Macintosh"; only its touch screen tells it apart.
  const ios = /iPhone|iPad|iPod/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1);
  const os = ios ? "ios" : /Android/.test(ua) ? "android" : /Macintosh|Mac OS X/.test(ua) ? "mac" : /Linux|X11/.test(ua) ? "linux" : null;
  if (os) {
    document.querySelector(`.dl[data-platform="${os}"]`)?.classList.add("suggested");
    const cta = document.querySelector("[data-cta]");
    const mine = cta?.querySelector(`[data-os="${os}"]`);
    if (cta && mine) {
      cta.querySelectorAll(".btn").forEach((b) => b.classList.replace("btn-primary", "btn-secondary"));
      mine.classList.replace("btn-secondary", "btn-primary");
      mine.classList.add("mine");
      cta.prepend(mine);
      // On a phone or tablet its own button replaces the "also for" line.
      if (mine.hasAttribute("data-mobile")) document.querySelector("[data-also]")?.setAttribute("hidden", "");
    }
    if (os !== "linux") selectTab(os);
  }

  // ---- Gallery tabs ----
  function selectTab(which) {
    document.querySelectorAll('[role="tab"]').forEach((t) => {
      const on = t.id === `tab-${which}`;
      t.setAttribute("aria-selected", String(on));
      document.getElementById(t.getAttribute("aria-controls")).hidden = !on;
    });
  }
  document.querySelectorAll('[role="tab"]').forEach((t) => t.addEventListener("click", () => selectTab(t.id.slice(4))));

  // ---- Lightbox ----
  const box = document.querySelector("dialog.lightbox");
  document.querySelectorAll(".gallery button").forEach((b) =>
    b.addEventListener("click", () => {
      const img = b.querySelector("img");
      box.querySelector("img").src = img.src;
      box.querySelector("img").alt = img.alt;
      box.querySelector("p").textContent = img.alt;
      box.showModal();
    }),
  );
  box?.addEventListener("click", (e) => { if (e.target === box) box.close(); });

  // ---- Copy buttons ----
  document.querySelectorAll("[data-copy]").forEach((b) =>
    b.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(b.dataset.copy);
        b.textContent = "Copied";
        setTimeout(() => (b.textContent = "Copy"), 1500);
      } catch (e) {}
    }),
  );
})();
