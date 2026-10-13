// SPDX-License-Identifier: GPL-2.0-or-later

#include "browser-relay.h"

#include <cstdio>
#include <memory>

#ifdef _WIN32
#include <io.h>
#include <winsock2.h>
#include <windows.h>
#endif

#include <QAbstractItemModel>
#include <QApplication>
#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QPointer>
#include <QQuickItem>
#include <QQuickWidget>
#include <QScopeGuard>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>

#include "browser-registration.h"
#include "gui/content/browsercapture.h"
#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

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
    // The registry fixture hides HKLM, including Winsock's provider catalog.
    // Load the TCP provider before installing its registry override.
    WSADATA winsock {};
    if (WSAStartup(MAKEWORD(2, 2), &winsock) != 0)
        return 2;
    const auto cleanupWinsock = qScopeGuard([] { WSACleanup(); });
    const SOCKET initialized = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (initialized == INVALID_SOCKET)
        return 2;
    closesocket(initialized);

    BrowserRegistrationFixture registration(profile.path());
    if (!registration.isReady())
        return 2;

    int result = 0;
    {
        Content::ContentView view(profile.path());
        view.initDownloads(QDir(profile.path()).filePath(QStringLiteral("downloads")));
        auto *controller = view.findChild<Qbutt::ContentController *>();
        if (!controller)
            return 2;
        QObject::connect(controller, &Qbutt::ContentController::browserChoiceRequested,
            &view, &Content::ContentView::setBrowserChoice);
        QObject::connect(controller, &Qbutt::ContentController::browserChanged, &view, [controller]
        {
            const QJsonObject state {
                {QStringLiteral("choice"), controller->property("browserChoice").toString()},
                {QStringLiteral("connection"), controller->property("browserConnectionState").toInt()},
                {QStringLiteral("setup"), controller->property("browserSetupState").toInt()},
                {QStringLiteral("failure"), controller->property("browserSetupFailure").toInt()},
                {QStringLiteral("connectedName"), controller->property("browserConnectedName").toString()},
                {QStringLiteral("extensionDirectory"), controller->property("browserExtensionDirectory").toString()}
            };
            send({{QStringLiteral("browserUi"), state}});
        });
        auto *model = qobject_cast<QAbstractItemModel *>(
            controller->property("downloads").value<QObject *>());
        if (!model)
            return 2;
        std::unique_ptr<QQuickWidget> downloadsPage {view.createDownloadsView(nullptr)};
        if (downloadsPage->status() != QQuickWidget::Ready || !downloadsPage->rootObject())
            return 2;
        QPointer<QQuickItem> downloadsList = downloadsPage->rootObject()->findChild<QQuickItem *>(
            QStringLiteral("contentDownloads"));
        if (!downloadsList)
            return 2;
        const auto downloadSnapshot = [controller, model, downloadsList]
        {
            if (!downloadsList)
                return;
            const auto roles = model->roleNames();
            const int idRole = roles.key("jobId", -1);
            const int statusRole = roles.key("status", -1);
            const int bytesRole = roles.key("bytes", -1);
            const int pathRole = roles.key("path", -1);
            QJsonArray jobs;
            for (int row = 0; row < model->rowCount(); ++row)
            {
                const QModelIndex index = model->index(row, 0);
                jobs.append(QJsonObject {
                    {QStringLiteral("id"), model->data(index, idRole).toString()},
                    {QStringLiteral("status"), model->data(index, statusRole).toString()},
                    {QStringLiteral("bytes"), model->data(index, bytesRole).toDouble()},
                    {QStringLiteral("path"), model->data(index, pathRole).toString()}
                });
            }
            send({{QStringLiteral("downloadSnapshot"), QJsonObject {
                {QStringLiteral("state"), controller->property("downloadsState").toString()},
                {QStringLiteral("pageCount"), downloadsList->property("count").toInt()},
                {QStringLiteral("jobs"), jobs}
            }}});
        };
        const auto changed = [&view, downloadSnapshot]
        {
            QTimer::singleShot(0, &view, downloadSnapshot);
        };
        QObject::connect(model, &QAbstractItemModel::rowsInserted, &view, changed);
        QObject::connect(model, &QAbstractItemModel::dataChanged, &view, changed);
        QObject::connect(model, &QAbstractItemModel::modelReset, &view, changed);
        QObject::connect(controller, &Qbutt::ContentController::downloadsStateChanged, &view, changed);
        changed();
        Content::BrowserCapture capture;
        QObject::connect(&capture, &Content::BrowserCapture::watchRequested, &view,
            [&view, &capture](const QUrl &page, const QString &title, const QString &captureId)
        {
            if (!view.watchCapture(page, title, captureId))
                capture.finishWatch(captureId, QStringLiteral("failed"));
        });
        QObject::connect(&capture, &Content::BrowserCapture::downloadRequested, &view,
            [&view, &capture](const QUrl &page, const QString &title, const QString &captureId)
        {
            if (!view.downloadCapture(page, title, captureId))
                capture.finishDownload(captureId, QStringLiteral("failed"));
        });
        QObject::connect(&capture, &Content::BrowserCapture::watchCancelled,
            &view, &Content::ContentView::cancelCaptureWatch);
        QObject::connect(&capture, &Content::BrowserCapture::downloadCancelled,
            &view, &Content::ContentView::cancelCaptureDownload);
        QObject::connect(&view, &Content::ContentView::capturedWatchFinished,
            &capture, &Content::BrowserCapture::finishWatch);
        QObject::connect(&view, &Content::ContentView::capturedDownloadFinished,
            &capture, &Content::BrowserCapture::finishDownload);
        QObject::connect(&view, &Content::ContentView::capturedWatchFinished, &view,
            [](const QString &captureId, const QString &reason)
        {
            const QJsonObject outcome {{QStringLiteral("captureId"), captureId},
                {QStringLiteral("reason"), reason}};
            send({{QStringLiteral("watchFinished"), outcome}});
        });
        QObject::connect(&view, &Content::ContentView::capturedDownloadFinished, &view,
            [](const QString &captureId, const QString &reason)
        {
            const QJsonObject outcome {{QStringLiteral("captureId"), captureId},
                {QStringLiteral("reason"), reason}};
            send({{QStringLiteral("downloadFinished"), outcome}});
        });
        QObject::connect(&view, &Content::ContentView::videoReady, &view,
            [](const quint64 launchToken, const QString &identity, const QUrl &url,
                const QUrl &, const QString &, const QString &, const QUrl &, const QString &,
                const QString &, const QString &, const QString &, const int, const QVector<QUrl> &)
        {
            const QJsonObject media {{QStringLiteral("launchToken"), static_cast<qint64>(launchToken)},
                {QStringLiteral("identity"), identity},
                {QStringLiteral("url"), url.toString(QUrl::FullyEncoded)}};
            send({{QStringLiteral("watchMedia"), media}});
        });
        QObject::connect(&capture, &Content::BrowserCapture::completed, &capture,
            [] { send({{QStringLiteral("captureCompleted"), true}}); });
        QObject::connect(&capture, &Content::BrowserCapture::error, &capture,
            [](const QString &message) { send({{QStringLiteral("captureError"), message}}); });
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
                else if (event == QStringLiteral("browserUiWatch"))
                    controller->watchBrowser();
                else if (event == QStringLiteral("browserUiClose"))
                    controller->closeBrowserSettings();
                else if (event == QStringLiteral("browserUiChoice"))
                    controller->chooseBrowser(data.value(QStringLiteral("choice")).toString());
                else if (event == QStringLiteral("browserUiSetup"))
                    controller->connectQcatch();
                else if (event == QStringLiteral("browserUiInstalled"))
                {
                    if (!registration.setInstalled(data.value(QStringLiteral("browser")).toString(),
                            data.value(QStringLiteral("installed")).toBool()))
                    {
                        qApp->exit(1);
                    }
                }
                else if (event == QStringLiteral("browserUiDefault"))
                {
                    if (!registration.setDefault(data.value(QStringLiteral("browser")).toString()))
                        qApp->exit(1);
                }
                else if (event == QStringLiteral("qcatchFixtureInstall"))
                {
                    if (!registration.setQcatchInstalled())
                        qApp->exit(1);
                }
                else if (event == QStringLiteral("qcatchFixtureFault"))
                    qputenv("QBUTT_QCATCH_SETUP_FIXTURE_FAIL", data.value(QStringLiteral("fault")).toString().toUtf8());
                else if (event == QStringLiteral("captureOpen"))
                    capture.open(QUrl(data.value(QStringLiteral("url")).toString(), QUrl::StrictMode));
                else if (event == QStringLiteral("captureCancel"))
                {
                    capture.cancel();
                    send({{QStringLiteral("captureCancelled"), true}});
                }
                else if (event == QStringLiteral("captureReleaseWatch"))
                    view.finishPlayback(data.value(QStringLiteral("launchToken")).toInteger());
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
        send({{QStringLiteral("ready"), true}, {QStringLiteral("captureProfile"), profile.path()}});
        result = qApp->exec();
    }
    if (!QThreadPool::globalInstance()->waitForDone(10'000))
        return 1;
    return result;
#endif
}
