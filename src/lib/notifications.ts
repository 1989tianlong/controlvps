import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

export async function sendSystemNotification(title: string, body: string) {
  let granted = await isPermissionGranted();
  if (!granted) granted = await requestPermission() === "granted";
  if (!granted) return false;
  sendNotification({ title, body });
  return true;
}
