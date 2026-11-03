// SPDX-License-Identifier: GPL-2.0-or-later
#include "cookies_linux.h"

#include <cmath>
#include <utility>

#include <QJsonDocument>
#include <QJsonObject>
#include <QWebEnginePage>
#include <QWebEngineProfile>
#include <QWebEngineUrlRequestInterceptor>

#include "request.h"

namespace
{
    constexpr int firstCommandId = 1073741824;
    constexpr qsizetype maxCookies = 512;
    constexpr qsizetype maxCommands = maxCookies + 1;
    constexpr qsizetype maxReplyBytes = 4 * 1024 * 1024;

    class FrontendRequests final : public QWebEngineUrlRequestInterceptor
    {
    public:
        explicit FrontendRequests(QObject *parent)
            : QWebEngineUrlRequestInterceptor(parent)
        {
        }

        void interceptRequest(QWebEngineUrlRequestInfo &request) override
        {
            const QString scheme = request.requestUrl().scheme();
            if (scheme != "devtools" && scheme != "qrc" && scheme != "data" && scheme != "blob")
                request.block(true);
        }
    };

    class CookieFrontend final : public QWebEnginePage
    {
    public:
        explicit CookieFrontend(QWebEngineProfile *profile)
            : QWebEnginePage(profile)
        {
        }

    protected:
        bool acceptNavigationRequest(const QUrl &url, NavigationType, bool) override
        {
            return url.scheme() == "devtools" || url == QUrl("about:blank");
        }

        QWebEnginePage *createWindow(WebWindowType) override
        {
            return nullptr;
        }

        void javaScriptConsoleMessage(JavaScriptConsoleMessageLevel, const QString &, int, const QString &) override
        {
        }
    };

    const QString installTransport = QStringLiteral(R"JS((() => {
        const host = globalThis.InspectorFrontendHost;
        const api = globalThis.InspectorFrontendAPI;
        if (!host || !api || typeof host.sendMessageToBackend !== 'function'
                || typeof api.dispatchMessage !== 'function')
            return false;
        const original = api.dispatchMessage.bind(api);
        const state = {pending: new Set(), replies: [], failed: false};
        api.dispatchMessage = message => {
            const reply = typeof message === 'string' ? JSON.parse(message) : message;
            if (!state.pending.delete(reply.id)) {
                original(message);
                return;
            }
            const bytes = typeof message === 'string' ? message.length : JSON.stringify(message).length;
            if (bytes > 4194304 || reply.error)
                state.failed = true;
            else
                state.replies.push(reply);
        };
        globalThis.qbuttCookieTransport = {
            send(commands) {
                state.pending = new Set(commands.map(command => command.id));
                state.replies = [];
                state.failed = false;
                for (const command of commands)
                    host.sendMessageToBackend(JSON.stringify(command));
            },
            take() {
                return state.pending.size ? null : {failed: state.failed, replies: state.replies};
            }
        };
        return true;
    })())JS");
}

SourceCookies::SourceCookies(QWebEnginePage *source, QObject *parent)
    : QObject(parent)
    , m_source(source)
    , m_profile(std::make_unique<QWebEngineProfile>())
    , m_frontend(std::make_unique<CookieFrontend>(m_profile.get()))
{
    m_profile->setUrlRequestInterceptor(new FrontendRequests(m_profile.get()));
    m_poll.setInterval(25);
    connect(&m_poll, &QTimer::timeout, this, &SourceCookies::poll);
    m_source->setDevToolsPage(m_frontend.get());
}

SourceCookies::~SourceCookies()
{
    m_stopping = true;
    m_poll.stop();
    m_completed = {};
    m_source->setDevToolsPage(nullptr);
    m_frontend.reset();
    m_profile.reset();
}

void SourceCookies::read(const QUrl &url, std::function<void(Snapshot)> completed)
{
    if (!SourceCheck::publicUrl(url))
    {
        completed(std::nullopt);
        return;
    }
    execute({QJsonObject {{"method", "Network.getCookies"},
        {"params", QJsonObject {{"urls", QJsonArray {url.toString(QUrl::FullyEncoded)}}}}}},
        [completed = std::move(completed)](Snapshot replies)
    {
        if (!replies || replies->size() != 1)
        {
            completed(std::nullopt);
            return;
        }
        const QJsonValue snapshot = replies->first().toObject().value("result").toObject().value("cookies");
        if (!snapshot.isArray() || snapshot.toArray().size() > maxCookies)
        {
            completed(std::nullopt);
            return;
        }
        QJsonArray cookies;
        for (const QJsonValue &value : snapshot.toArray())
        {
            QJsonObject cookie = value.toObject();
            // The native HTTP client has no partition key. Leave partitioned
            // cookies with Chromium rather than exporting a broader identity.
            if (cookie.contains("partitionKey") || cookie.value("partitionKeyOpaque").toBool())
                continue;
            const QString domain = cookie.value("domain").toString();
            const double expires = cookie.value("expires").toDouble();
            if (!cookie.value("name").isString() || !cookie.value("value").isString() || domain.isEmpty()
                || !cookie.value("path").isString() || !cookie.value("secure").isBool()
                || !cookie.value("httpOnly").isBool() || !cookie.value("session").isBool()
                || (!cookie.value("session").toBool() && (!std::isfinite(expires) || expires <= 0)))
            {
                completed(std::nullopt);
                return;
            }
            QJsonObject entry {{"name", cookie.value("name")}, {"value", cookie.value("value")},
                {"domain", domain}, {"path", cookie.value("path")}, {"secure", cookie.value("secure")},
                {"httpOnly", cookie.value("httpOnly")}, {"hostOnly", !domain.startsWith(u'.')}};
            if (!cookie.value("session").toBool())
                entry.insert("expires", expires);
            cookies.append(entry);
        }
        completed(cookies);
    });
}

void SourceCookies::apply(const QUrl &url, const QJsonArray &seeds, const QJsonArray &removals,
    std::function<void(bool)> completed)
{
    QJsonArray commands;
    for (const QJsonValue &value : removals)
    {
        const QJsonObject cookie = value.toObject();
        commands.append(QJsonObject {{"method", "Network.deleteCookies"}, {"params", QJsonObject {
            {"name", cookie.value("name")}, {"domain", cookie.value("domain")}, {"path", cookie.value("path")}}}});
    }
    if (!seeds.isEmpty())
    {
        QJsonArray cookies;
        for (const QJsonValue &value : seeds)
        {
            QJsonObject cookie = value.toObject();
            cookie.remove("domain");
            cookie.remove("hostOnly");
            cookie.insert("url", url.toString(QUrl::FullyEncoded));
            cookies.append(cookie);
        }
        commands.append(QJsonObject {{"method", "Network.setCookies"}, {"params", QJsonObject {{"cookies", cookies}}}});
    }
    if (commands.isEmpty())
    {
        completed(true);
        return;
    }
    execute(std::move(commands), [completed = std::move(completed)](Snapshot replies) { completed(replies.has_value()); });
}

void SourceCookies::execute(QJsonArray commands, std::function<void(Snapshot)> completed)
{
    if (m_completed || commands.isEmpty() || commands.size() > maxCommands)
    {
        completed(std::nullopt);
        return;
    }
    for (qsizetype index = 0; index < commands.size(); ++index)
    {
        QJsonObject command = commands.at(index).toObject();
        command.insert("id", firstCommandId + index);
        commands[index] = command;
    }
    m_commands = std::move(commands);
    m_completed = std::move(completed);
    m_sent = false;
    m_poll.start();
    poll();
}

void SourceCookies::poll()
{
    if (m_busy || !m_completed)
        return;
    m_busy = true;
    if (!m_ready)
    {
        m_frontend->runJavaScript(installTransport, [this](const QVariant &value)
        {
            if (m_stopping)
                return;
            m_busy = false;
            m_ready = value.toBool();
        });
    }
    else if (!m_sent)
    {
        const QString script = QStringLiteral("qbuttCookieTransport.send(%1)")
            .arg(QString::fromUtf8(QJsonDocument(m_commands).toJson(QJsonDocument::Compact)));
        m_frontend->runJavaScript(script, [this](const QVariant &)
        {
            if (m_stopping)
                return;
            m_busy = false;
            m_sent = true;
        });
    }
    else
    {
        m_frontend->runJavaScript(QStringLiteral("qbuttCookieTransport.take()"), [this](const QVariant &value)
        {
            if (m_stopping)
                return;
            m_busy = false;
            const QJsonObject state = QJsonObject::fromVariantMap(value.toMap());
            if (state.isEmpty())
                return;
            if (state.value("failed").toBool() || !state.value("replies").isArray()
                || QJsonDocument(state).toJson(QJsonDocument::Compact).size() > maxReplyBytes)
            {
                finish(std::nullopt);
            }
            else
            {
                const QJsonArray replies = state.value("replies").toArray();
                finish(replies.size() == m_commands.size() ? Snapshot(replies) : std::nullopt);
            }
        });
    }
}

void SourceCookies::finish(Snapshot replies)
{
    m_poll.stop();
    m_commands = {};
    auto completed = std::exchange(m_completed, {});
    completed(std::move(replies));
}
