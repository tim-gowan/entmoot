package mini;

public final class Sibling {
    private Sibling() {}

    public static String help(String value) {
        return value == null ? "" : value.trim();
    }
}
