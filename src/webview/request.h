// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <optional>

#include <QJsonArray>
#include <QJsonObject>
#include <QSet>
#include <QUrl>

namespace SourceCheck
{
    inline const QUrl proxyCheckUrl {QStringLiteral("http://qbutt-webview.invalid/")};
    inline constexpr char documentState[] = R"JS((() => {
        const has = selector => document.querySelector(selector) !== null;
        const anubis = has('script#anubis_challenge[type="application/json"]')
            && has('script#anubis-main[src*="/.within.website/x/cmd/anubis/"]');
        const cloudflare = has('#challenge-form, #cf-challenge-running, #challenge-stage, .cf-turnstile');
        const ddg = has('script[src*="check.ddos-guard.net/check.js"]');
        const captcha = has(`form#captcha-form, form[action*="/captcha"], input[name="cap_sid"],
            input[name="g-recaptcha-response"], textarea[name="g-recaptcha-response"],
            input[name="h-captcha-response"], .g-recaptcha, .h-captcha`);
        const login = location.pathname === '/forum/login.php'
            || (has('input[name="login_username"]') && has('input[name="login_password"]')
                && !has('#logged-in-username'));
        return {url: location.href, ready: document.readyState === 'complete',
            challenge: anubis || cloudflare || ddg || captcha || login};
    })())JS";


    struct Request
    {
        QString id;
        QUrl url;
        QUrl proxy;
        QString proxyUsername;
        QString proxyPassword;
        QString profileDirectory;
        QString configuredSession;
        QJsonArray initialCookies;
        QSet<QString> retainedCookies;
        int timeoutMs = 0;
        bool authoritativeCookies = false;
        bool interactive = false;
    };

    bool publicUrl(const QUrl &url);
    std::optional<Request> parseRequest(const QJsonObject &request);
}
