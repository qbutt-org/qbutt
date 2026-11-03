// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <functional>
#include <memory>

#include <QJsonObject>
#include <QList>
#include <QNetworkCookie>
#include <QObject>
#include <QTimer>

#include "request.h"

class QWebEngineProfile;
class QWebEngineView;

class WebSession final : public QObject
{
public:
    explicit WebSession(std::function<void(const QJsonObject &)> completed, QObject *parent = nullptr);
    ~WebSession() override;
    void start(const QJsonObject &request);
    void cancel();
    QString id() const;

private:
    void prepareProfile();
    void synchronizeCookies();
    void cookieBarrier();
    void authenticateProxy();
    void pollDocument();
    void collectSession(const QUrl &url);
    void finish(const QJsonObject &result);
    void fail(const QString &code);

    std::function<void(const QJsonObject &)> m_completed;
    SourceCheck::Request m_request;
    std::unique_ptr<QWebEngineProfile> m_profile;
    std::unique_ptr<QWebEngineView> m_view;
    QList<QNetworkCookie> m_cookies;
    QTimer m_deadline;
    QTimer m_poll;
    QByteArray m_barrierName;
    enum class CookiePhase
    {
        Loading,
        Applying
    };
    CookiePhase m_cookiePhase = CookiePhase::Loading;
    bool m_profileUnchanged = false;
    bool m_started = false;
    bool m_finished = false;
    bool m_proxyAuthenticated = false;
    bool m_loaded = false;
    bool m_polling = false;
    bool m_sawChallenge = false;
    int m_cleanPolls = 0;
    int m_httpStatus = 0;
    quint64 m_generation = 0;
};
