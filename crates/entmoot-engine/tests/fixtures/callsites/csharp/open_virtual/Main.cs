public class Base { public virtual int Leaf() { return 1; } }
public static class Main {
    public static int Caller(Base value) { return value.Leaf(); }
}
