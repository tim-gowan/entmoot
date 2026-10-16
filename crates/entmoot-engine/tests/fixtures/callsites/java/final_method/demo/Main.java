package demo;
public class Main {
    public final int leaf() { return 1; }
    public int caller() { return leaf(); }
}
