package demo;

public class Caller {
    // Unicode offset sentinel: 😀
    public static String run(String value) { return Sibling.help(value); }
    public static void leaf() {}
    public static String virtual(Worker worker, String value) { return worker.help(value); }
}
interface Worker { String help(String value); }
