// Browser-only stand-in for @tauri-apps/plugin-opener (see core.ts).
export async function openUrl(url: string): Promise<void> {
  console.log("mock openUrl", url);
}
