package mini;

import java.util.function.Function;

public final class Caller {
    private Caller() {}

    public static String run(String value) {
        return Sibling.help(value);
    }

    public static String indirect(Function<String, String> callback, String value) {
        return callback.apply(value);
    }
}
