public class Base { public virtual int Leaf() { return 1; } }
public class Child : Base { public sealed override int Leaf() { return 2; } }
public static class Main {
    public static int Caller(Child child) { return child.Leaf(); }
}
