package demo;
interface Worker { default int work() { return 7; } }
public final class Main {
    static int caller(Worker worker) { return worker.work(); }
}
