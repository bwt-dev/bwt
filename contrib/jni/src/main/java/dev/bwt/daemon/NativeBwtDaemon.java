package dev.bwt.daemon;

public class NativeBwtDaemon {
    static {
        System.loadLibrary("bwt");
    }

    // Start the bwt daemon with the configured server(s)
    // Blocks the current thread until the daemon is stopped.
    // Returns a pointer to be used with shutdown().
    public static native long start(String jsonConfig, CallbackNotifier callback);

    // Shutdown thw bwt daemon
    public static native void shutdown(long shutdownPtr);

    // Test the Bitcoin Core RPC connection details
    // Throws an exception on failures.
    public static native void testRpc(String jsonConfig);
}
