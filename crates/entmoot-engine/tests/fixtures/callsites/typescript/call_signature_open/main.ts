type Fn = { (): number };
export function caller(fn: Fn): number { return fn(); }
