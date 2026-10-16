package demo;
public final class Main {
    private int leaf() { return 1; }
    public int caller() { return leaf(); }
}
