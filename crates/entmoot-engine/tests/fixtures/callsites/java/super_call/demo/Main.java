package demo;
class Base { int leaf() { return 1; } }
public class Main extends Base {
    @Override int leaf() { return 2; }
    int caller() { return super.leaf(); }
}
