namespace Mini;

public static class Caller
{
    public static string Run(string value) => Sibling.Help(value);

    public static void Indirect(System.Action callback) => callback();

    public static Sibling Make() => new Sibling();
}
