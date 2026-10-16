public class Base { public virtual int Leaf() { return 1; } }
public class Child : Base {
    public override int Leaf() { return 2; }
    public int Caller() { return base.Leaf(); }
}
