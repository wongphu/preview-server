// Injected by preview-server: reloads the page when files change.
(() => {
  if (window.__previewServer) return;
  window.__previewServer = true;

  // Re-fetch same-origin stylesheets in place. The old <link> is removed only
  // after the new one loads, so the page never flashes unstyled.
  const swapStylesheets = () => {
    const links = document.querySelectorAll('link[rel="stylesheet"]');
    for (const link of links) {
      const url = new URL(link.href, location.href);
      if (url.origin !== location.origin) continue;
      url.searchParams.set("__preview", Date.now());
      const next = link.cloneNode();
      next.href = url.href;
      next.onload = next.onerror = () => link.remove();
      link.after(next);
    }
  };

  let lostConnection = false;

  const connect = () => {
    const source = new EventSource("/__preview/events");

    source.addEventListener("open", () => {
      // The server restarted while we were away; files may have changed.
      if (lostConnection) location.reload();
    });

    source.addEventListener("change", (event) => {
      const paths = JSON.parse(event.data);
      const cssOnly = paths.length > 0 && paths.every((p) => p.endsWith(".css"));
      if (cssOnly && document.querySelector('link[rel="stylesheet"]')) {
        console.log("[preview] css changed:", paths.join(", "));
        swapStylesheets();
      } else {
        location.reload();
      }
    });

    source.addEventListener("error", () => {
      lostConnection = true;
      // EventSource retries on its own unless the connection was closed for good.
      if (source.readyState === EventSource.CLOSED) {
        source.close();
        setTimeout(connect, 1000);
      }
    });
  };

  connect();
})();
