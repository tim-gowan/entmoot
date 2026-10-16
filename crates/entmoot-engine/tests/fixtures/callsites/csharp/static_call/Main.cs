public static class Main {
    public static int Leaf() { return 1; }
    public static int Caller() { return Leaf(); }
}
