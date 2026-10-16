struct Base { virtual int leaf() { return 1; } };
int caller(Base& b) { return b.leaf(); }
