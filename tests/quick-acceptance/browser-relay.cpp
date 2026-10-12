// SPDX-License-Identifier: GPL-2.0-or-later
#include "browser-relay.h"
#include "browser-registration.h"

#include <cstdio>

#include <QApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>

#ifdef Q_OS_WIN
#include <io.h>
#include <windows.h>
#endif

#include "gui/quick/contentview.h"

namespace
{
    constexpr qsizetype MAX_LINE = 768 * 1024 + 1024;

    void send(const QJsonObject &message)
    {
        const QByteArray line = QJsonDocument(message).toJson(QJsonDocument::Compact) + '\n';
        fwrite(line.constData(), 1, line.size(), stdout);
        fflush(stdout);
    }
}

int runBrowserRelayHost()
{
#ifndef Q_OS_WIN
    return 2;
#else
    // This explicit process fixture never selects a user's discovery or profile.
    const QString discovery = qEnvironmentVariable("QBUTT_BROWSER_BRIDGE_DISCOVERY");
    QTemporaryDir profile;
    if (discovery.isEmpty() || !profile.isValid())
        return 2;
    BrowserRegistrationFixture registration(profile.path());
    if (!registration.isReady())
        return 2;

    int result = 0;
    {
        Content::ContentView view(profile.path());
        QObject::connect(&view, &Content::ContentView::browserReply, &view,
            [](const quint64 generation, const quint64 id, const QByteArray &reply)
        {
            QJsonObject response = QJsonDocument::fromJson(reply).object();
            response.insert(QStringLiteral("id"), static_cast<qint64>(id));
            send({{QStringLiteral("generation"), static_cast<qint64>(generation)},
                {QStringLiteral("browserReply"), response}});
        });
        QByteArray input;
        const HANDLE pipe = reinterpret_cast<HANDLE>(_get_osfhandle(_fileno(stdin)));
        QTimer reader;
        QObject::connect(&reader, &QTimer::timeout, &view, [&]
        {
            DWORD available = 0;
            if (!PeekNamedPipe(pipe, nullptr, 0, nullptr, &available, nullptr))
            {
                qApp->quit();
                return;
            }
            if (!available)
                return;
            if (available + input.size() > MAX_LINE * 2)
            {
                qApp->exit(1);
                return;
            }
            QByteArray bytes(available, '\0');
            DWORD count = 0;
            if (!ReadFile(pipe, bytes.data(), available, &count, nullptr))
            {
                qApp->exit(1);
                return;
            }
            input.append(bytes.constData(), count);
            qsizetype end = 0;
            while ((end = input.indexOf('\n')) >= 0)
            {
                const QJsonDocument document = QJsonDocument::fromJson(input.first(end));
                input.remove(0, end + 1);
                if (!document.isObject() || end > MAX_LINE)
                {
                    qApp->exit(1);
                    return;
                }
                const QJsonObject message = document.object();
                const quint64 generation = message.value(QStringLiteral("generation")).toInteger(1);
                const QString event = message.value(QStringLiteral("event")).toString();
                const QJsonObject data = message.value(QStringLiteral("data")).toObject();
                const quint64 id = data.value(QStringLiteral("id")).toInteger();
                if (event == QStringLiteral("browserRequest"))
                    view.relayBrowserRequest(generation, id, QJsonDocument(data).toJson(QJsonDocument::Compact));
                else if (event == QStringLiteral("browserCancel"))
                    view.cancelBrowserRequest(generation, id);
                else if (event == QStringLiteral("browserSessionClosed"))
                    view.closeBrowserSession(generation);
                else if (event == QStringLiteral("browserInitialize"))
                    view.initializeBrowser();
                else if (event == QStringLiteral("browserRegistration"))
                {
                    if (!registration.setRegistered(data.value(QStringLiteral("browser")).toString(),
                            data.value(QStringLiteral("registered")).toBool()))
                    {
                        qApp->exit(1);
                    }
                }
                else
                    qApp->exit(1);
            }
            if (input.size() > MAX_LINE)
                qApp->exit(1);
        });
        reader.start(10);
        QTimer::singleShot(180'000, &view, [] { qApp->exit(1); });
        send({{QStringLiteral("ready"), true}});
        result = qApp->exec();
    }
    if (!QThreadPool::globalInstance()->waitForDone(10'000))
        return 1;
    return result;
#endif
}
