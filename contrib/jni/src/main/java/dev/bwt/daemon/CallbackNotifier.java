package dev.bwt.daemon;

public interface CallbackNotifier {
    void onBooting();
    void onSyncProgress(float progress, int tip);
    void onScanProgress(float progress, int eta);
    void onElectrumReady(String addr);
    void onHttpReady(String addr);
}