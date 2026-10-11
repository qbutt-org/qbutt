// SPDX-License-Identifier: GPL-2.0-or-later
#include "client.h"

#include <utility>

#include <QCoreApplication>
#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QTimer>

#include "base/bittorrent/session.h"
#include "base/global.h"
#include "base/path.h"
#include "base/profile.h"

namespace
{
    constexpr qsizetype MAX_MESSAGE_SIZE = 16 * 1024 * 1024;
    constexpr qsizetype MAX_PENDING_REQUESTS = 128;

    QJsonObject localizeFailure(QJsonObject failure)
    {
        const QString code = failure.value(u"code"_s).toString();
        QString message;
        if (code == u"qplay_unavailable")
            message = Content::Client::tr("qplay is unavailable or damaged. Reinstall qplay or run the qbutt installer again.");
        else if (code == u"qplay_incompatible")
            message = Content::Client::tr("This version of qplay is incompatible. Install a compatible qplay version.");
        else if (code == u"qcatch_unavailable")
            message = Content::Client::tr("qcatch setup did not complete. Reinstall qcatch or run the qbutt installer again.");
        else if (code == u"qcatch_incompatible")
            message = Content::Client::tr("This version of qcatch is incompatible. Install a compatible qcatch version.");
        else if (code == u"component_download_failed")
            message = Content::Client::tr("Could not download the component. Check your connection and try again.");
        else if (code == u"component_integrity_failed")
            message = Content::Client::tr("The component failed its integrity check. Reinstall qbutt and try again.");
        else if (code == u"capture_expired")
            message = Content::Client::tr("This capture expired or changed. Choose the media again in qcatch.");
        else if (code == u"capture_session")
            message = Content::Client::tr("This qcatch selection belongs to the current browser session. Share the source page instead.");
        else if (code == u"unsupported_multi_audio")
            message = Content::Client::tr("Choose one audio track in qcatch before sending this media to qbutt.");
        else if (code == u"unsupported_private_tracks")
            message = Content::Client::tr("These audio or subtitle tracks require separate sign-in. Download them through qcatch instead.");
        else if (code == u"library_limit")
            message = Content::Client::tr("The saved variants exceed 16 MiB. Remove this item from the library and save it again.");
        else if (code == u"browser_required")
            message = Content::Client::tr("Install qcatch and connect the extension in the selected browser.");
        else if (code == u"browser_update_required")
            message = Content::Client::tr("Update qcatch and reload its browser extension to search VK Video.");
        else if (code == u"browser_busy")
            message = Content::Client::tr("The browser is busy. Try again in a few seconds.");
        else if (code == u"browser_timeout")
            message = Content::Client::tr("The browser took too long. Open the source and try again.");
        else if (code == u"selected_content_changed")
            message = Content::Client::tr("The source changed the selected page. Choose it again and retry.");
        else if (code == u"browser_error")
            message = Content::Client::tr("Could not read the source in the browser. Open the source and try again.");
        else if (code == u"browser_choice_required")
            message = Content::Client::tr("Choose an installed browser in Sources.");
        else if (code == u"browser_unsupported")
            message = Content::Client::tr("qcatch supports Microsoft Edge and Google Chrome.");
        else if (code == u"webview_unavailable")
            message = Content::Client::tr("The source check is unavailable. Reinstall qbutt.");
        else if (code == u"webview_runtime_required")
            message = Content::Client::tr("Install Microsoft Edge WebView2 Runtime to check this source.");
        else if (code == u"webview_timeout")
            message = Content::Client::tr("The source check took too long. Try again.");
        else if (code == u"webview_failed")
            message = Content::Client::tr("Could not check the source. Try again.");
        else if (code == u"browser_check_required")
            message = Content::Client::tr("This source requires a browser check.");
        else if (code == u"source_session_invalid")
            message = Content::Client::tr("Check the source session in Sources.");
        else if (code == u"source_authorization_required")
            message = Content::Client::tr("Save your account session in Sources before using this source.");
        else if (code == u"ai_unavailable")
            message = Content::Client::tr("Codex is unavailable. Install it and sign in, or choose another AI service in Sources.");
        else if (code == u"ai_authentication")
            message = Content::Client::tr("Sign in to your AI service or check its API key in Sources.");
        else if (code == u"ai_configuration")
            message = Content::Client::tr("Check the AI translation settings in Sources.");
        else if (code == u"ai_timeout")
            message = Content::Client::tr("AI translation took too long. Retry.");
        else if (code == u"ai_provider")
            message = Content::Client::tr("The AI service could not complete the translation. Retry or check its settings in Sources.");
        else if (code == u"ai_invalid-response")
            message = Content::Client::tr("The AI service returned an invalid translation. Retry or choose another model.");
        else if (code == u"ai_capability")
            message = Content::Client::tr("This AI service cannot translate with the current settings. Choose another service or model in Sources.");
        if (!message.isEmpty())
            failure.insert(u"message"_s, message);
        return failure;
    }
}

Content::Client *Content::Client::m_instance = nullptr;

Content::Client *Content::Client::instance()
{
    if (!m_instance)
        m_instance = new Client(QCoreApplication::instance());
    return m_instance;
}

void Content::Client::freeInstance()
{
    delete std::exchange(m_instance, nullptr);
}

Content::Client::Client(QObject *parent)
    : QObject(parent)
{
    connect(&m_process, &QProcess::started, this, &Client::flushRequests);
    connect(&m_process, &QProcess::readyReadStandardOutput, this, &Client::readOutput);
    connect(&m_process, &QProcess::readyReadStandardError, this, [this]
    {
        // Provider diagnostics can contain private URLs. Consume without logging them.
        m_process.readAllStandardError();
    });
    connect(&m_process, &QProcess::errorOccurred, this, [this](QProcess::ProcessError)
    {
        failPending(tr("The content service could not start: %1").arg(m_process.errorString()));
    });
    connect(&m_process, &QProcess::finished, this, [this]
    {
        readOutput();
        failPending(tr("The content service stopped. Try again."));
    });
}

Content::Client::~Client()
{
    disconnect(&m_process, nullptr, this, nullptr);
    if (m_process.state() != QProcess::NotRunning)
    {
        flushRequests();
        m_process.closeWriteChannel();
        if (!m_process.waitForFinished(3000))
        {
            m_process.kill();
            m_process.waitForFinished(1000);
        }
    }
}

qint64 Content::Client::request(const QString &method, const QJsonObject &params)
{
    const qint64 id = ++m_nextId;
    const QByteArray message = QJsonDocument(QJsonObject {
        {u"id"_s, id}, {u"method"_s, method}, {u"params"_s, params}
    }).toJson(QJsonDocument::Compact) + '\n';

    if ((m_pending.size() >= MAX_PENDING_REQUESTS) || (message.size() > MAX_MESSAGE_SIZE))
    {
        QTimer::singleShot(0, this, [this, id]
        {
            emit eventReceived(id, u"error"_s, QJsonObject {{u"message"_s, tr("Too many requests or content is too large.")}});
        });
        return id;
    }

    m_pending.insert(id);
    m_queue.insert(id, message);
    QTimer::singleShot(0, this, [this]
    {
        if (m_queue.isEmpty())
            return;

        if (m_process.state() == QProcess::NotRunning)
        {
            m_output.clear();
            const QString dataDir = QDir(Profile::instance()->location(SpecialFolder::Data).toString()).filePath(u"content"_s);
            m_process.start(QDir(QCoreApplication::applicationDirPath()).filePath(u"qbutt-content.exe"_s),
                {u"--data-dir"_s, dataDir, u"--download-dir"_s, BitTorrent::Session::instance()->savePath().toString()});
        }
        else if (m_process.state() == QProcess::Running)
        {
            flushRequests();
        }
    });
    const int timeout = ((method == u"reader.translate") || (method == u"reader.ahead")) ? 1800000
        : (method == u"connectBrowser") ? 600000 : 180000;
    QTimer::singleShot(timeout, this, [this, id, method]
    {
        if (!m_pending.remove(id))
            return;

        m_queue.remove(id);
        if (method != u"cancel")
            cancel(id);
        emit eventReceived(id, u"error"_s, QJsonObject {{u"message"_s, tr("The source took too long. Try again.")}});
    });
    return id;
}

void Content::Client::cancel(const qint64 id)
{
    if (id > 0)
        request(u"cancel"_s, {{u"id"_s, id}});
}

void Content::Client::flushRequests()
{
    const auto queue = std::exchange(m_queue, {});
    for (const QByteArray &message : queue)
        m_process.write(message);
}

void Content::Client::readOutput()
{
    m_output += m_process.readAllStandardOutput();
    while (!m_output.isEmpty())
    {
        const qsizetype end = m_output.indexOf('\n');
        if ((end > MAX_MESSAGE_SIZE) || ((end < 0) && (m_output.size() > MAX_MESSAGE_SIZE)))
        {
            m_process.kill();
            failPending(tr("The content service returned too much data."));
            return;
        }
        if (end < 0)
            return;

        QJsonParseError error;
        const QJsonDocument document = QJsonDocument::fromJson(m_output.first(end), &error);
        m_output.remove(0, end + 1);
        const QJsonObject object = document.object();
        const qint64 id = object.value(u"id"_s).toInteger();
        const QString event = object.value(u"event"_s).toString();
        if ((error.error != QJsonParseError::NoError) || !document.isObject() || event.isEmpty())
        {
            m_process.kill();
            failPending(tr("The content service returned an invalid response."));
            return;
        }
        QJsonValue data = object.value(u"data"_s);
        if ((event == u"error") || ((event == u"complete") && data.isObject())
            || ((event == u"job") && (data.toObject().value(u"status"_s).toString() == u"error")))
            data = localizeFailure(data.toObject());
        else if (event == u"jobs")
        {
            QJsonArray jobs;
            for (const QJsonValue &job : data.toArray())
                jobs.append(localizeFailure(job.toObject()));
            data = jobs;
        }
        else if (event == u"sources")
        {
            QJsonArray sources;
            for (const QJsonValue &value : data.toArray())
            {
                QJsonObject source = value.toObject();
                if (source.value(u"health"_s).isObject())
                    source.insert(u"health"_s, localizeFailure(source.value(u"health"_s).toObject()));
                sources.append(source);
            }
            data = sources;
        }
        if ((id == 0) && ((event == u"job") || (event == u"progress")))
        {
            emit eventReceived(id, event, data);
            continue;
        }
        if (!m_pending.contains(id))
            continue;

        if ((event == u"complete") || (event == u"error"))
            m_pending.remove(id);
        emit eventReceived(id, event, data);
    }
}

void Content::Client::failPending(const QString &message)
{
    m_queue.clear();
    m_output.clear();
    const QSet<qint64> pending = std::exchange(m_pending, {});
    for (const qint64 id : pending)
        emit eventReceived(id, u"error"_s, QJsonObject {{u"message"_s, message}});
}
