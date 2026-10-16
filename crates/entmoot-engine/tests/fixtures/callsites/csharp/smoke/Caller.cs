namespace Demo;

public static class Caller {
    // Unicode offset sentinel: 😀
    public static string Run(string value) { return Sibling.Help(value); }
    public static string Indirect(System.Func<string,string> callback, string value) { return callback(value); }
    public static void Leaf() {}
    public static string Virtual(IWorker worker, string value) { return worker.Help(value); }
}
public interface IWorker { string Help(string value); }
