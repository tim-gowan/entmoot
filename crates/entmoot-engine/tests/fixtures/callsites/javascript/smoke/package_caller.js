import { debounce } from "lodash";

export function usesPackage(value) {
  return debounce(() => value, 0);
}
