// Size and date strings in the style GNOME Files uses (GLib's g_format_size
// and Nautilus' "regular" date format).

/** Decimal units like GLib: "512 bytes", "4.1 kB", "12.3 MB". */
export function formatSize(bytes: number): string {
  if (bytes === 1) return "1 byte";
  if (bytes < 1000) return `${bytes} bytes`;
  const units = ["kB", "MB", "GB", "TB", "PB"];
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

/** "12 items" / "1 item" / "Empty", as shown in the Size column for folders. */
export function formatItemCount(count: number): string {
  if (count === 0) return "Empty";
  return count === 1 ? "1 item" : `${count.toLocaleString()} items`;
}

const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const weekdayFmt = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const dayMonthFmt = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });
const fullFmt = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });
const detailFmt = new Intl.DateTimeFormat(undefined, { dateStyle: "full", timeStyle: "short" });

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** Today → "14:32", yesterday → "Yesterday", this week → "Mon", this year → "3 Oct", else "3 Oct 2024". */
export function formatDate(unixSeconds: number, now = new Date()): string {
  if (unixSeconds === 0) return "";
  const date = new Date(unixSeconds * 1000);
  const days = Math.round((startOfDay(now) - startOfDay(date)) / 86_400_000);
  if (days === 0) return timeFmt.format(date);
  if (days === 1) return "Yesterday";
  if (days > 1 && days < 7) return weekdayFmt.format(date);
  if (date.getFullYear() === now.getFullYear()) return dayMonthFmt.format(date);
  return fullFmt.format(date);
}

/** Long form for tooltips. */
export function formatDateDetailed(unixSeconds: number): string {
  return unixSeconds === 0 ? "" : detailFmt.format(new Date(unixSeconds * 1000));
}
