# Rust loads this class and calls run through JNI. R8 cannot see those uses.
-keep class dev.timeslay.day.notes.NotesStorage {
    public static java.lang.String run(android.content.Context, java.lang.String);
}
