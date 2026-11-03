// SPDX-License-Identifier: GPL-2.0-or-later
#include "request.h"

#include <cmath>

#include <QDateTime>
#include <QDir>
#include <QFileInfo>
#include <QHostAddress>
#include <QRegularExpression>

using namespace SourceCheck;

std::optional<Request> SourceCheck::parseRequest(const QJsonObject &request)
{
    Request result;
    result.id = request.value("id").toString();
    result.url = QUrl {request.value("url").toString(), QUrl::StrictMode};
    const QString mode = request.value("mode").toString();
    result.interactive = (mode == "interactive");
    result.includeDocument = request.value("includeDocument").toBool();
    const QString profile = QDir::cleanPath(request.value("profileDirectory").toString());
    const QJsonObject proxy = request.value("proxy").toObject();
    result.proxy = QUrl {proxy.value("url").toString(), QUrl::StrictMode};
    result.proxyUsername = proxy.value("username").toString();
    result.proxyPassword = proxy.value("password").toString();
    const int timeout = request.value("timeoutMs").toInt();
    const QFileInfo profileInfo {profile};
    const QRegularExpression profileName {"\\A[a-f0-9]{64}\\z"};
    const QString fingerprint = request.value("configuredSession").toString();
    const QJsonArray initialCookies = request.value("initialCookies").toArray();
    const bool authoritative = request.value("authoritativeCookies").toBool();
    const QJsonArray retainedCookies = request.value("retainedCookies").toArray();
    if (result.id.isEmpty() || result.id.size() > 128 || !publicUrl(result.url)
        || (mode != "silent" && mode != "interactive")
        || (request.contains("includeDocument") && !request.value("includeDocument").isBool())
        || timeout < 1000 || timeout > (result.interactive ? 180000 : 30000)
        || !profileInfo.isAbsolute() || profile.startsWith("//")
        || !profileName.match(profileInfo.fileName()).hasMatch()
        || QFileInfo {profileInfo.path()}.fileName() != "web-session"
        || result.proxy.scheme() != "http" || result.proxy.host() != "127.0.0.1" || result.proxy.port() <= 0
        || !result.proxy.userInfo().isEmpty() || !result.proxy.path().isEmpty() || result.proxy.hasQuery() || result.proxy.hasFragment()
        || result.proxyUsername != "qbutt" || result.proxyPassword.size() < 32 || result.proxyPassword.size() > 128)
    {
        return std::nullopt;
    }

    const bool hasConfiguredSession = request.contains("configuredSession") || request.contains("initialCookies");
    if ((hasConfiguredSession && (!profileName.match(fingerprint).hasMatch()
            || !request.value("initialCookies").isArray() || initialCookies.size() > 128))
        || (request.contains("authoritativeCookies") && (!request.value("authoritativeCookies").isBool() || !hasConfiguredSession))
        || (request.contains("retainedCookies") && (!authoritative || !request.value("retainedCookies").isArray()
            || retainedCookies.size() > 128)))
    {
        return std::nullopt;
    }
    const QRegularExpression cookieName {"\\A[!#$%&'*+.^_`|~0-9A-Za-z-]+\\z"};
    const QRegularExpression cookieValue {"\\A[\\x21\\x23-\\x2B\\x2D-\\x3A\\x3C-\\x5B\\x5D-\\x7E]*\\z"};
    const auto validIdentity = [&result, &cookieName](const QJsonValue &value)
    {
        const QJsonObject cookie = value.toObject();
        const QString name = cookie.value("name").toString();
        return value.isObject() && name.size() <= 256 && cookieName.match(name).hasMatch()
            && cookie.value("domain").toString() == result.url.host(QUrl::EncodeUnicode) && cookie.value("path").toString() == "/"
            && cookie.value("hostOnly").isBool() && cookie.value("hostOnly").toBool();
    };
    QSet<QString> names;
    for (const QJsonValue &value : initialCookies)
    {
        const QJsonObject cookie = value.toObject();
        const QString name = cookie.value("name").toString();
        const QString content = cookie.value("value").toString();
        QString octets = content;
        if (octets.size() >= 2 && octets.startsWith('"') && octets.endsWith('"'))
            octets = octets.mid(1, octets.size() - 2);
        if (!validIdentity(value)
            || !cookie.value("value").isString() || name.size() + content.size() > 4096
            || !cookieValue.match(octets).hasMatch() || names.contains(name)
            || !cookie.value("secure").isBool()
            || !cookie.value("httpOnly").isBool()
            || (cookie.contains("expires") && (!cookie.value("expires").isDouble()
                || !std::isfinite(cookie.value("expires").toDouble())
                || cookie.value("expires").toDouble() <= QDateTime::currentSecsSinceEpoch())))
        {
            return std::nullopt;
        }
        names.insert(name);
    }
    QSet<QString> retained;
    for (const QJsonValue &cookie : retainedCookies)
    {
        const QString name = cookie.toObject().value("name").toString();
        if (!validIdentity(cookie) || names.contains(name))
        {
            return std::nullopt;
        }
        names.insert(name);
        retained.insert(name);
    }

    result.profileDirectory = profile;
    result.configuredSession = fingerprint;
    result.initialCookies = initialCookies;
    result.retainedCookies = retained;
    result.authoritativeCookies = authoritative;
    result.timeoutMs = timeout;
    return result;
}

bool SourceCheck::publicUrl(const QUrl &url)
{
    if (!url.isValid() || (url.scheme() != "https" && url.scheme() != "http")
        || url.host().isEmpty() || url.host() == proxyCheckUrl.host()
        || !url.userInfo().isEmpty() || url.port(443) <= 0)
        return false;
    const QHostAddress address {url.host()};
    if (address.isNull())
        return true;
    bool isV4 = false;
    const quint32 ip = address.toIPv4Address(&isV4);
    if (isV4)
    {
        const quint32 first = ip >> 24;
        const quint32 second = (ip >> 16) & 255;
        return first != 0 && first != 10 && first != 127 && first < 224
            && !(first == 169 && second == 254)
            && !(first == 172 && second >= 16 && second <= 31)
            && !(first == 192 && second == 168)
            && !(first == 100 && second >= 64 && second <= 127);
    }
    return (address.toIPv6Address()[0] & 0xE0) == 0x20;
}
