// Browser-only stand-in for @tauri-apps/plugin-opener (see core.ts).
export async function openPath(path: string): Promise<void> {
  if (path.endsWith(".mp3")) throw "No application knows how to open this file";
  console.log("mock openPath", path);
}

export async function openUrl(url: string): Promise<void> {
  console.log("mock openUrl", url);
}
