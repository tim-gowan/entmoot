package demo;
public final class Main {
    public static int leaf() { return 1; }
    public static int caller() { return leaf(); }
}
