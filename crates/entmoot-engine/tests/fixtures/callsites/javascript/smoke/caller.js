export function help(value) {
  return String(value).trim();
}

export function caller(value) {
  return help(value);
}

export function indirect(callback, value) {
  return callback(value);
}

export function leaf() {}
