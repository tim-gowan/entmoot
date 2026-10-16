export const api = { leaf() { return 1; } };
export function caller() { return api.leaf(); }
