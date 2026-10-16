public class Box {
    public int Leaf() { return 1; }
}
public static class Main {
    public static int Caller(Box box) { return box.Leaf(); }
}
