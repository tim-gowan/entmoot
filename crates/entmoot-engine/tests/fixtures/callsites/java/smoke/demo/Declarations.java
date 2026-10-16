package demo;

abstract class AbstractWorker {
    abstract int work();
    native int nativeWork();
}
interface DefaultWorker {
    default int work() { return 7; }
}
class Declarations {
    static int abstractCall(AbstractWorker worker) { return worker.work(); }
    static int nativeCall(AbstractWorker worker) { return worker.nativeWork(); }
    static int defaultCall(DefaultWorker worker) { return worker.work(); }
}
