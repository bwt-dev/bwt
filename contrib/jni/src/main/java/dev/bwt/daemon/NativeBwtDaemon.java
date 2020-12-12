package dev.bwt.daemon;

public class NativeBwtDaemon {
    static {
        System.loadLibrary("bwt");
    }

    // Start the bwt daemon with the configured server(s)
    // Blocks until the initial indexing is completed and the servers are ready.
    // Returns a pointer to be used with shutdown().
    public static native long start(String jsonConfig, CallbackNotifier callback);

    // Shutdown thw bwt daemon
    public static native void shutdown(long shutdownPtr);
}