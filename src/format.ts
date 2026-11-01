export function formatByteSize(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "—";
  const units = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${new Intl.NumberFormat("uk-UA", { maximumFractionDigits: unit === 0 ? 0 : 1 }).format(size)} ${units[unit]}`;
}
