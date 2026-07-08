// Runs inside the hidden capture WebviewWindow.
// Pulls the rendered template HTML from Rust, replaces the document with it,
// waits for fonts/layout, then tells Rust the page is ready to snapshot.
// The actual pixel capture happens on the Rust side via WKWebView's native
// takeSnapshot API: the JS-side html-to-image path is unusable in WKWebView —
// drawing an SVG that contains <foreignObject> taints the canvas, so
// toDataURL/getImageData throw SecurityError (and img.decode() additionally
// hangs forever in hidden windows).
import { invoke } from "@tauri-apps/api/core";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const log = (message: string) =>
  invoke("spike_log", { message }).catch(() => {});

async function main() {
  log(
    `page loaded, viewport ${window.innerWidth}x${window.innerHeight}, dpr ${window.devicePixelRatio}`,
  );
  const payload = await invoke<{ html: string }>("get_capture_payload");

  // Replace the entire document with the rendered template. The window object
  // (and Tauri's IPC globals) survives document.write, so `invoke` keeps
  // working afterwards.
  document.open();
  document.write(payload.html);
  document.close();

  // Wait for fonts + a settle delay for layout/paint. Deliberately NOT
  // requestAnimationFrame: WKWebView suspends rAF in hidden windows.
  await document.fonts.ready;
  await sleep(300);
  log(`ready to snapshot, body ${document.body.offsetWidth}x${document.body.offsetHeight}`);

  await invoke("notify_capture_ready", {
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    devicePixelRatio: window.devicePixelRatio,
  });
}

main().catch((e) => {
  invoke("submit_capture_error", { message: String(e) }).catch(() => {});
});
