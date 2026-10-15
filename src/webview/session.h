#pragma once

#include <functional>

#include <windows.h>
#include <wrl.h>
#include <WebView2.h>

#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QSet>
#include <QString>
#include <QTimer>
#include <QUrl>

class WebSession final : public QObject
{
public:
    explicit WebSession(std::function<void(const QJsonObject &)> completed, QObject *parent = nullptr);
    ~WebSession() override;
    void start(const QJsonObject &request);
    void cancel();
    QString id() const;

private:
    static LRESULT CALLBACK windowProcedure(HWND window, UINT message, WPARAM wParam, LPARAM lParam);
    bool configure();
    void prepareSession(const QJsonArray &cookies, const QString &fingerprint, const QString &profile,
        bool authoritative, const QSet<QString> &retained);
    bool seedCookies(ICoreWebView2CookieManager *manager, const QJsonArray &cookies);
    void authenticateProxy();
    void navigateSource();
    void pollDocument();
    void collectSession(const QUrl &url);
    void finish(const QJsonObject &result);
    void fail(const QString &code);
    void close();
    static bool publicUrl(const QUrl &url);

    std::function<void(const QJsonObject &)> m_completed;
    QString m_id;
    QUrl m_url;
    QUrl m_proxy;
    QString m_proxyUsername;
    QString m_proxyPassword;
    QString m_userAgent;
    HWND m_window = nullptr;
    Microsoft::WRL::ComPtr<ICoreWebView2Environment> m_environment;
    Microsoft::WRL::ComPtr<ICoreWebView2Controller> m_controller;
    Microsoft::WRL::ComPtr<ICoreWebView2> m_view;
    QTimer m_deadline;
    QTimer m_poll;
    bool m_started = false;
    bool m_finished = false;
    bool m_loaded = false;
    bool m_polling = false;
    bool m_sawChallenge = false;
    bool m_interactive = false;
    bool m_proxyAuthenticated = false;
    int m_cleanPolls = 0;
    int m_httpStatus = 0;
    quint64 m_navigationGeneration = 0;
    UINT64 m_navigationId = 0;
};
