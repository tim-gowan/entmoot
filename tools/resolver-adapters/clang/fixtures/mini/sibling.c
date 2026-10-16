/* Mini sibling for the Clang sidecar fixture. */
int sibling_help(const char *value) {
    (void)value;
    return 1;
}

int sibling_indirect(int (*callback)(const char *), const char *value) {
    return callback(value);
}
