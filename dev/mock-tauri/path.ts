// Browser-only stand-in for @tauri-apps/api/path (see core.ts).
const home = "/Users/me";
export const homeDir = async () => home;
export const desktopDir = async () => `${home}/Desktop`;
export const documentDir = async () => `${home}/Documents`;
export const downloadDir = async () => `${home}/Downloads`;
export const audioDir = async () => `${home}/Music`;
export const pictureDir = async () => `${home}/Pictures`;
export const videoDir = async () => `${home}/Movies`;
export const publicDir = async () => `${home}/Public`;
export const templateDir = async () => { throw "not supported on macOS"; };
