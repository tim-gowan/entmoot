export function help(value: string): string {
  return String(value).trim();
}

export function caller(value: string): string {
  return help(value);
}

export function indirect(callback: (value: string) => string, value: string): string {
  return callback(value);
}

export function leaf(): void {}
