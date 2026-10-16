package demo;
class Base { int leaf() { return 1; } }
public final class Main {
    static int caller(Base value) { return value.leaf(); }
}
