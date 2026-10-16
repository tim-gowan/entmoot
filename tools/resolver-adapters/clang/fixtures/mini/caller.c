/* Mini caller for the Clang sidecar fixture. */
int sibling_help(const char *value);
int sibling_indirect(int (*callback)(const char *), const char *value);

#define WRAP_HELP(v) sibling_help(v)

int caller_run(const char *value) {
    return sibling_help(value);
}

int caller_indirect(int (*callback)(const char *), const char *value) {
    return sibling_indirect(callback, value);
}

int caller_macro(const char *value) {
    return WRAP_HELP(value);
}
