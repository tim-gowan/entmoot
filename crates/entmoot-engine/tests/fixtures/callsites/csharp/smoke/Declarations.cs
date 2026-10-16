namespace Demo;

public abstract class AbstractWorker {
    public abstract int Work();
    [System.Runtime.InteropServices.DllImport("fixture-native")]
    public static extern int Native();
}
public interface IDefaultWorker {
    int Work() { return 7; }
}
public static partial class Declarations {
    public static partial int Compute(int value);
    public static int AbstractCall(AbstractWorker worker) { return worker.Work(); }
    public static int NativeCall() { return AbstractWorker.Native(); }
    public static int DefaultCall(IDefaultWorker worker) { return worker.Work(); }
    public static int PartialCall() { return Compute(7); }
    public static int ExpressionCall() { return Expression(); }
    public static int Expression() => 7;
}
