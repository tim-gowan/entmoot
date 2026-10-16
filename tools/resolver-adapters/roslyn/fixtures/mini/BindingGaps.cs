namespace Mini;

/// Independently authored binding-failure controls. MissingName has no symbol;
/// dual-null Overload is ambiguous. Neither establishes runtime dispatch.
public static class BindingGaps
{
    public static int Overload(string a, object b) => 0;
    public static int Overload(object a, string b) => 1;
    public static int Ambiguous() => Overload(null, null);
    public static int Missing() => MissingName(1);
}
