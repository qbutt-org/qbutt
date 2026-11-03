// SPDX-License-Identifier: GPL-2.0-or-later
#include "session_linux.h"

#include "cookies_linux.h"

#include <atomic>
#include <utility>

#include <sys/stat.h>
#include <unistd.h>

#include <QAuthenticator>
#include <QCloseEvent>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QNetworkProxy>
#include <QSaveFile>
#include <QShortcut>
#include <QWebEngineCertificateError>
#include <QWebEngineClientCertificateSelection>
#include <QWebEngineDownloadRequest>
#include <QWebEngineFileSystemAccessRequest>
#include <QWebEngineLoadingInfo>
#include <QWebEnginePage>
#include <QWebEnginePermission>
#include <QWebEngineProfile>
#include <QWebEngineSettings>
#include <QWebEngineUrlRequestInterceptor>
#include <QWebEngineView>

using namespace SourceCheck;

namespace
{
    class SourceRequests final : public QWebEngineUrlRequestInterceptor
    {
    public:
        explicit SourceRequests(QObject *parent)
            : QWebEngineUrlRequestInterceptor(parent)
        {
        }

        void interceptRequest(QWebEngineUrlRequestInfo &request) override
        {
            const bool bootstrap = !authenticated.load() && request.requestUrl() == proxyCheckUrl
                && request.resourceType() == QWebEngineUrlRequestInfo::ResourceTypeMainFrame;
            if (!bootstrap && !publicUrl(request.requestUrl()))
                request.block(true);
        }

        std::atomic_bool authenticated = false;
    };

    class SourcePage final : public QWebEnginePage
    {
    public:
        SourcePage(QWebEngineProfile *profile, std::function<bool(const QUrl &, bool)> navigate, QObject *parent)
            : QWebEnginePage(profile, parent)
            , m_navigate(std::move(navigate))
        {
        }

    protected:
        bool acceptNavigationRequest(const QUrl &url, NavigationType, const bool mainFrame) override
        {
            return m_navigate(url, mainFrame);
        }

        QWebEnginePage *createWindow(WebWindowType) override
        {
            return nullptr;
        }

        QStringList chooseFiles(FileSelectionMode, const QStringList &, const QStringList &) override
        {
            return {};
        }

        void javaScriptAlert(const QUrl &, const QString &) override
        {
        }

        bool javaScriptConfirm(const QUrl &, const QString &) override
        {
            return false;
        }

        bool javaScriptPrompt(const QUrl &, const QString &, const QString &, QString *) override
        {
            return false;
        }

        void javaScriptConsoleMessage(JavaScriptConsoleMessageLevel, const QString &, int, const QString &) override
        {
        }

    private:
        std::function<bool(const QUrl &, bool)> m_navigate;
    };

    class SourceWindow final : public QWebEngineView
    {
    public:
        explicit SourceWindow(std::function<void()> cancel)
            : m_cancel(std::move(cancel))
        {
        }

    protected:
        void closeEvent(QCloseEvent *event) override
        {
            event->ignore();
            m_cancel();
        }

    private:
        std::function<void()> m_cancel;
    };

    bool ownedDirectory(const QString &path)
    {
        struct stat status {};
        const QByteArray encoded = QFile::encodeName(path);
        return lstat(encoded.constData(), &status) == 0 && S_ISDIR(status.st_mode)
            && status.st_uid == getuid() && (status.st_mode & 0077) == 0;
    }


}

WebSession::WebSession(std::function<void(const QJsonObject &)> completed, QObject *parent)
    : QObject(parent)
    , m_completed(std::move(completed))
{
    m_deadline.setSingleShot(true);
    m_poll.setInterval(250);
    connect(&m_deadline, &QTimer::timeout, this, [this]
    {
        fail(m_sawChallenge ? QStringLiteral("browser_check_required") : QStringLiteral("webview_timeout"));
    });
    connect(&m_poll, &QTimer::timeout, this, &WebSession::pollDocument);
}

WebSession::~WebSession()
{
    m_cookies.reset();
    m_view.reset();
    m_profile.reset();
}

QString WebSession::id() const
{
    return m_request.id;
}

void WebSession::start(const QJsonObject &request)
{
    if (m_finished)
        return;
    if (m_started)
    {
        fail(QStringLiteral("webview_failed"));
        return;
    }
    m_started = true;
    const auto parsed = parseRequest(request);
    if (!parsed)
    {
        m_request.id = request.value("id").toString();
        m_request.url = QUrl(request.value("url").toString(), QUrl::StrictMode);
        fail(QStringLiteral("webview_failed"));
        return;
    }
    m_request = *parsed;
    m_deadline.start(m_request.timeoutMs);
    prepareProfile();
}

void WebSession::prepareProfile()
{
    if (QFileInfo(m_request.profileDirectory).isSymLink() || !QDir().mkpath(m_request.profileDirectory)
        || !QFile::setPermissions(m_request.profileDirectory, QFile::ReadOwner | QFile::WriteOwner | QFile::ExeOwner)
        || !ownedDirectory(m_request.profileDirectory))
    {
        fail(QStringLiteral("webview_failed"));
        return;
    }
    const QString markerPath = m_request.profileDirectory + QStringLiteral("/qbutt-configured-session");
    QFile marker(markerPath);
    if (!m_request.configuredSession.isEmpty() && marker.exists())
    {
        if (QFileInfo(marker).isSymLink() || !marker.open(QIODevice::ReadOnly))
        {
            fail(QStringLiteral("webview_failed"));
            return;
        }
        m_profileUnchanged = marker.read(65) == m_request.configuredSession.toLatin1();
        marker.close();
        if (!m_profileUnchanged && !marker.remove())
        {
            fail(QStringLiteral("webview_failed"));
            return;
        }
    }
    if (!m_request.configuredSession.isEmpty() && !m_profileUnchanged)
    {
        // Reset only this helper's two owned Chromium directories. Removing the
        // marker first makes an interrupted reset repeat on the next check.
        for (const QString &name : {QStringLiteral("storage"), QStringLiteral("cache")})
        {
            const QString path = QDir(m_request.profileDirectory).filePath(name);
            if (QFileInfo::exists(path) && (!ownedDirectory(path) || !QDir(path).removeRecursively()))
            {
                fail(QStringLiteral("webview_failed"));
                return;
            }
        }
    }

    QNetworkProxy::setApplicationProxy(QNetworkProxy(QNetworkProxy::HttpProxy, m_request.proxy.host(), m_request.proxy.port()));
    m_profile = std::make_unique<QWebEngineProfile>(QStringLiteral("qbutt-source"));
    m_profile->setPersistentStoragePath(m_request.profileDirectory + QStringLiteral("/storage"));
    m_profile->setCachePath(m_request.profileDirectory + QStringLiteral("/cache"));
    m_profile->setPersistentCookiesPolicy(QWebEngineProfile::ForcePersistentCookies);
    m_profile->setPersistentPermissionsPolicy(QWebEngineProfile::PersistentPermissionsPolicy::AskEveryTime);
    auto requests = new SourceRequests(m_profile.get());
    m_profile->setUrlRequestInterceptor(requests);
    m_view = std::make_unique<SourceWindow>([this] { cancel(); });
    m_view->resize(960, 640);
    m_view->setWindowTitle(m_request.url.host() + QStringLiteral(" [qbutt]"));
    m_view->setContextMenuPolicy(Qt::NoContextMenu);
    auto cancelShortcut = new QShortcut(QKeySequence(Qt::Key_Escape), m_view.get());
    cancelShortcut->setContext(Qt::WidgetWithChildrenShortcut);
    connect(cancelShortcut, &QShortcut::activated, this, &WebSession::cancel);
    auto page = new SourcePage(m_profile.get(), [this](const QUrl &url, const bool mainFrame)
    {
        if (!mainFrame)
            return url == QUrl(QStringLiteral("about:blank")) || publicUrl(url);
        if ((!m_proxyAuthenticated && url != proxyCheckUrl) || (m_proxyAuthenticated && !publicUrl(url)))
        {
            fail(QStringLiteral("webview_failed"));
            return false;
        }
        m_loaded = false;
        m_cleanPolls = 0;
        ++m_generation;
        return true;
    }, m_view.get());
    m_view->setPage(page);
    page->setAudioMuted(true);
    page->settings()->setAttribute(QWebEngineSettings::PluginsEnabled, false);
    page->settings()->setAttribute(QWebEngineSettings::PdfViewerEnabled, false);
    page->settings()->setAttribute(QWebEngineSettings::JavascriptCanOpenWindows, false);
    page->settings()->setAttribute(QWebEngineSettings::JavascriptCanAccessClipboard, false);
    page->settings()->setAttribute(QWebEngineSettings::LocalContentCanAccessRemoteUrls, false);
    page->settings()->setAttribute(QWebEngineSettings::LocalContentCanAccessFileUrls, false);
    page->settings()->setAttribute(QWebEngineSettings::ErrorPageEnabled, false);
    page->settings()->setAttribute(QWebEngineSettings::ScreenCaptureEnabled, false);
    page->settings()->setAttribute(QWebEngineSettings::WebRTCPublicInterfacesOnly, true);
    connect(page, &QWebEnginePage::permissionRequested, this, [](QWebEnginePermission permission) { permission.deny(); });
    connect(page, &QWebEnginePage::fileSystemAccessRequested, this,
        [](QWebEngineFileSystemAccessRequest access) { access.reject(); });
    connect(page, &QWebEnginePage::selectClientCertificate, this,
        [](QWebEngineClientCertificateSelection selection) { selection.selectNone(); });
    connect(page, &QWebEnginePage::certificateError, this,
        [](QWebEngineCertificateError error) { error.rejectCertificate(); });
    connect(m_profile.get(), &QWebEngineProfile::downloadRequested, this,
        [](QWebEngineDownloadRequest *download) { download->cancel(); });
    connect(page, &QWebEnginePage::renderProcessTerminated, this, [this] { fail(QStringLiteral("webview_failed")); });
    connect(page, &QWebEnginePage::proxyAuthenticationRequired, this,
        [this](const QUrl &, QAuthenticator *authenticator, const QString &host)
    {
        if (host == m_request.proxy.host() && authenticator->realm() == QStringLiteral("qbutt"))
        {
            authenticator->setUser(m_request.proxyUsername);
            authenticator->setPassword(m_request.proxyPassword);
        }
    });
    connect(page, &QWebEnginePage::loadingChanged, this, [this, requests](const QWebEngineLoadingInfo &info)
    {
        if (m_finished || info.status() == QWebEngineLoadingInfo::LoadStartedStatus)
            return;
        if (info.status() == QWebEngineLoadingInfo::LoadStoppedStatus)
            return;
        const bool success = info.status() == QWebEngineLoadingInfo::LoadSucceededStatus;
        m_httpStatus = info.errorDomain() == QWebEngineLoadingInfo::HttpStatusCodeDomain ? info.errorCode() : 200;
        if (!m_proxyAuthenticated)
        {
            if (!success || info.url() != proxyCheckUrl || m_httpStatus != 200)
            {
                fail(QStringLiteral("webview_failed"));
                return;
            }
            m_proxyAuthenticated = true;
            requests->authenticated.store(true);
            m_cookies = std::make_unique<SourceCookies>(m_view->page());
            m_cookies->read(m_request.url, [this](SourceCookies::Snapshot cookies)
            {
                if (m_finished)
                    return;
                if (!cookies)
                    fail(QStringLiteral("webview_failed"));
                else
                    synchronizeCookies(*cookies);
            });
        }
        else if (success || m_httpStatus >= 400)
        {
            m_loaded = true;
        }
        else
        {
            fail(QStringLiteral("webview_failed"));
        }
    });

    // This nonforwarded page authenticates the gateway before the internal
    // cookie adapter can inspect the source profile or navigate to a source.
    m_view->load(proxyCheckUrl);
}

void WebSession::synchronizeCookies(const QJsonArray &existing)
{
    QSet<QString> supplied;
    QSet<QString> present;
    QJsonArray removals;
    for (const QJsonValue &entry : m_request.initialCookies)
        supplied.insert(entry.toObject().value("name").toString());
    for (const QJsonValue &entry : existing)
    {
        const QJsonObject cookie = entry.toObject();
        if (cookie.value("domain").toString() != m_request.url.host(QUrl::EncodeUnicode)
            || cookie.value("path").toString() != QStringLiteral("/"))
        {
            continue;
        }
        const QString name = cookie.value("name").toString();
        present.insert(name);
        if (m_request.authoritativeCookies && !supplied.contains(name) && !m_request.retainedCookies.contains(name))
            removals.append(cookie);
    }
    QJsonArray seeds;
    for (const QJsonValue &entry : m_request.initialCookies)
    {
        if (!m_profileUnchanged || m_request.authoritativeCookies || !present.contains(entry.toObject().value("name").toString()))
            seeds.append(entry);
    }
    m_cookies->apply(m_request.url, seeds, removals, [this](const bool success)
    {
        if (m_finished)
            return;
        if (!success)
            fail(QStringLiteral("webview_failed"));
        else
            navigateSource();
    });
}

void WebSession::navigateSource()
{
    if (!m_request.configuredSession.isEmpty() && !m_profileUnchanged)
    {
        QSaveFile marker(m_request.profileDirectory + QStringLiteral("/qbutt-configured-session"));
        const QByteArray fingerprint = m_request.configuredSession.toLatin1();
        if (!marker.open(QIODevice::WriteOnly) || marker.write(fingerprint) != fingerprint.size() || !marker.commit())
        {
            fail(QStringLiteral("webview_failed"));
            return;
        }
    }
    if (m_request.interactive)
        m_view->show();
    m_view->load(m_request.url);
    m_poll.start();
}

void WebSession::pollDocument()
{
    if (m_finished || !m_loaded || m_polling)
        return;
    m_polling = true;
    const quint64 generation = m_generation;
    m_view->page()->runJavaScript(QString::fromUtf8(documentState), [this, generation](const QVariant &value)
    {
        m_polling = false;
        if (m_finished || !m_loaded || generation != m_generation)
            return;
        const QJsonObject state = QJsonObject::fromVariantMap(value.toMap());
        const QUrl url(state.value("url").toString(), QUrl::StrictMode);
        if (!publicUrl(url))
        {
            fail(QStringLiteral("webview_failed"));
            return;
        }
        if (state.value("challenge").toBool() || m_httpStatus == 401 || m_httpStatus == 403)
        {
            m_sawChallenge = true;
            m_cleanPolls = 0;
        }
        else if (state.value("ready").toBool() && ++m_cleanPolls >= 2)
        {
            if (m_httpStatus >= 400)
                fail(QStringLiteral("webview_failed"));
            else
                collectSession(url);
        }
    });
}

void WebSession::collectSession(const QUrl &url)
{
    m_poll.stop();
    const quint64 generation = m_generation;
    m_cookies->read(url, [this, url, generation](SourceCookies::Snapshot cookies)
    {
        if (m_finished)
            return;
        if (generation != m_generation)
        {
            m_poll.start();
            return;
        }
        if (!cookies)
        {
            fail(QStringLiteral("webview_failed"));
            return;
        }
        finish({{"session", QJsonObject {{"url", url.toString(QUrl::FullyEncoded)},
            {"userAgent", m_profile->httpUserAgent()}, {"cookies", *cookies}}}});
    });
}

void WebSession::cancel()
{
    fail(QStringLiteral("canceled"));
}

void WebSession::fail(const QString &code)
{
    finish({{"error", QJsonObject {{"code", code}, {"url", m_request.url.toString(QUrl::FullyEncoded)}}}});
}

void WebSession::finish(const QJsonObject &result)
{
    if (m_finished)
        return;
    m_finished = true;
    m_deadline.stop();
    m_poll.stop();
    QTimer::singleShot(0, this, [this, result]
    {
        m_cookies.reset();
        m_view.reset();
        m_profile.reset();
        QJsonObject response = result;
        response.insert("id", m_request.id);
        if (QJsonDocument(response).toJson(QJsonDocument::Compact).size() >= 256 * 1024)
            response = {{"id", m_request.id}, {"error", QJsonObject {{"code", "webview_failed"},
                {"url", m_request.url.toString(QUrl::FullyEncoded)}}}};
        m_completed(response);
    });
}
