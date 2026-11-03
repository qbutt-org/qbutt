// SPDX-License-Identifier: GPL-2.0-or-later
#include "qbutt_quick/native.h"

#include <QDir>
#include <QDesktopServices>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QSettings>
#include <QStandardPaths>
#include <QUrl>

#include "qbutt_quick/src/controller.cxxqt.h"

#ifdef Q_OS_LINUX
extern "C"
{
    typedef struct _GAppInfo GAppInfo;
    GAppInfo *g_app_info_get_default_for_uri_scheme(const char *scheme);
    const char *g_app_info_get_executable(GAppInfo *application);
    void g_object_unref(void *object);
}
#endif

namespace
{
    Qbutt::NativeHostState manifestState(const QString &path)
    {
        if (path.isEmpty() || !QFileInfo::exists(path))
            return Qbutt::NativeHostState::Missing;
        QFile manifest(path);
        if (!manifest.open(QIODevice::ReadOnly))
            return Qbutt::NativeHostState::Unknown;
        if (manifest.size() > 64 * 1024)
            return Qbutt::NativeHostState::Missing;
        const QJsonObject value = QJsonDocument::fromJson(manifest.readAll()).object();
        const QFileInfo executable {value.value(QStringLiteral("path")).toString()};
        if ((value.value(QStringLiteral("name")).toString() == QStringLiteral("org.qbutt.qcatch"))
            && (value.value(QStringLiteral("type")).toString() == QStringLiteral("stdio"))
            && executable.isAbsolute() && executable.isFile()
#ifndef Q_OS_WIN
            && executable.isExecutable()
#endif
            && value.value(QStringLiteral("allowed_origins")).toArray().contains(
                QStringLiteral("chrome-extension://jnmnnglmejlbjnjmmjampnhfncpcbffh/")))
        {
            return Qbutt::NativeHostState::Registered;
        }
        return Qbutt::NativeHostState::Missing;
    }

#ifdef Q_OS_WIN
    QString browserPath(const bool edge)
    {
        const QString executable = edge ? QStringLiteral("msedge.exe") : QStringLiteral("chrome.exe");
        for (const QString &hive : {QStringLiteral("HKEY_CURRENT_USER"), QStringLiteral("HKEY_LOCAL_MACHINE")})
        {
            const QSettings application {hive + QStringLiteral("\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\")
                + executable, QSettings::NativeFormat};
            const QString path = application.value(QStringLiteral("Default")).toString();
            if (QDir::isAbsolutePath(path) && QFileInfo(path).isFile())
                return path;
        }
        return {};
    }

    Qbutt::NativeHostState nativeHostState(const bool edge)
    {
        auto state = Qbutt::NativeHostState::Missing;
        const QString browserKey = edge ? QStringLiteral("Microsoft\\Edge") : QStringLiteral("Google\\Chrome");
        for (const QString &hive : {QStringLiteral("HKEY_CURRENT_USER"), QStringLiteral("HKEY_LOCAL_MACHINE")})
        {
            const QString key = hive + QStringLiteral("\\Software\\") + browserKey
                + QStringLiteral("\\NativeMessagingHosts\\org.qbutt.qcatch");
            const QSettings registration {key, QSettings::NativeFormat};
            const QString path = registration.value(QStringLiteral("Default")).toString();
            if (registration.status() != QSettings::NoError)
            {
                state = Qbutt::NativeHostState::Unknown;
                continue;
            }
            const Qbutt::NativeHostState observed = manifestState(path);
            if (observed == Qbutt::NativeHostState::Registered)
                return observed;
            if (observed == Qbutt::NativeHostState::Unknown)
                state = observed;
        }
        return state;
    }
#endif
#ifdef Q_OS_LINUX
    QString defaultBrowser()
    {
        GAppInfo *application = g_app_info_get_default_for_uri_scheme("https");
        if (!application)
            return {};
        const QString executable = QString::fromLocal8Bit(g_app_info_get_executable(application));
        g_object_unref(application);
        return QStandardPaths::findExecutable(executable);
    }

    QString browserPath(const bool edge)
    {
        const QStringList names = edge
            ? QStringList {QStringLiteral("microsoft-edge"), QStringLiteral("microsoft-edge-stable")}
            : QStringList {QStringLiteral("google-chrome"), QStringLiteral("google-chrome-stable"),
                QStringLiteral("chromium"), QStringLiteral("chromium-browser")};
        const QString preferred = defaultBrowser();
        if (names.contains(QFileInfo(preferred).fileName()))
        {
            const QString path = QStandardPaths::findExecutable(preferred);
            if (!path.isEmpty())
                return path;
        }
        for (const QString &name : names)
        {
            const QString path = QStandardPaths::findExecutable(name);
            if (!path.isEmpty())
                return path;
        }
        return {};
    }

    Qbutt::NativeHostState nativeHostState(const bool edge)
    {
        const QDir config {QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation)};
        const QStringList directories = edge ? QStringList {QStringLiteral("microsoft-edge")}
            : QStringList {QStringLiteral("google-chrome"), QStringLiteral("chromium")};
        QStringList paths;
        for (const QString &directory : directories)
            paths.append(config.filePath(directory + QStringLiteral("/NativeMessagingHosts/org.qbutt.qcatch.json")));
        paths.append(edge ? QStringLiteral("/etc/opt/edge/native-messaging-hosts/org.qbutt.qcatch.json")
            : QStringLiteral("/etc/opt/chrome/native-messaging-hosts/org.qbutt.qcatch.json"));
        if (!edge)
            paths.append(QStringLiteral("/etc/chromium/native-messaging-hosts/org.qbutt.qcatch.json"));

        auto state = Qbutt::NativeHostState::Missing;
        for (const QString &path : paths)
        {
            const Qbutt::NativeHostState observed = manifestState(path);
            if (observed == Qbutt::NativeHostState::Registered)
                return observed;
            if (observed == Qbutt::NativeHostState::Unknown)
                state = observed;
        }
        return state;
    }

#endif
}

::rust::Vec<Qbutt::BrowserInstallation> Qbutt::browserInstallations()
{
    ::rust::Vec<BrowserInstallation> browsers;
#ifdef Q_OS_WIN
    const QString choiceKey = QStringLiteral("HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\https\\UserChoice");
    const QSettings defaults {choiceKey, QSettings::NativeFormat};
    const QString progId = defaults.value(QStringLiteral("ProgId")).toString();
    for (const QString &id : {QStringLiteral("edge"), QStringLiteral("chrome")})
    {
        const bool edge = id == QStringLiteral("edge");
        const bool installed = !browserPath(edge).isEmpty();
        browsers.push_back({id, edge ? QStringLiteral("Microsoft Edge") : QStringLiteral("Google Chrome"),
            progId.startsWith(edge ? QStringLiteral("MSEdgeHTM") : QStringLiteral("ChromeHTML"), Qt::CaseInsensitive),
            installed, nativeHostState(edge)});
    }
#elif defined(Q_OS_LINUX)
    const QString defaultPath = QFileInfo(defaultBrowser()).canonicalFilePath();
    for (const QString &id : {QStringLiteral("edge"), QStringLiteral("chrome")})
    {
        const bool edge = id == QStringLiteral("edge");
        const QString executable = browserPath(edge);
        QString name = edge ? QStringLiteral("Microsoft Edge") : QStringLiteral("Google Chrome");
        if (!edge && QFileInfo(executable).fileName().startsWith(QStringLiteral("chromium")))
            name = QStringLiteral("Chromium");
        const bool isDefault = !defaultPath.isEmpty() && QFileInfo(executable).canonicalFilePath() == defaultPath;
        browsers.push_back({id, name, isDefault, !executable.isEmpty(), nativeHostState(edge)});
    }
#endif
    return browsers;
}

QString Qbutt::qcatchExecutable()
{
#ifdef Q_OS_WIN
    const QSettings installation {QStringLiteral("HKEY_CURRENT_USER\\Software\\qbutt-org\\qcatch"),
        QSettings::NativeFormat};
    const QString directory = installation.value(QStringLiteral("InstallDir")).toString();
    if (!QDir::isAbsolutePath(directory))
        return {};

    const QFileInfo executable {QDir(directory).filePath(QStringLiteral("qcatch.exe"))};
    if (executable.isFile() && !executable.isSymLink())
        return executable.absoluteFilePath();
#elif defined(Q_OS_LINUX)
    const QDir data {QStandardPaths::writableLocation(QStandardPaths::GenericDataLocation)};
    const QFileInfo installed {data.filePath(QStringLiteral("qcatch/app/qcatch"))};
    if (installed.isFile() && installed.isExecutable())
        return installed.canonicalFilePath();
    return QFileInfo(QStandardPaths::findExecutable(QStringLiteral("qcatch"))).canonicalFilePath();
#endif
    return {};
}

QString Qbutt::browserDiscoveryPath()
{
    const QString override = qEnvironmentVariable("QBUTT_BROWSER_BRIDGE_DISCOVERY");
    if (!override.isEmpty())
        return override;

    const QDir localData {QStandardPaths::writableLocation(QStandardPaths::GenericDataLocation)};
    return localData.filePath(QStringLiteral("qbutt/content/browser-bridge.json"));
}

QString Qbutt::qcatchSetupCacheDirectory()
{
    const QDir cache {QStandardPaths::writableLocation(QStandardPaths::GenericCacheLocation)};
    return cache.filePath(QStringLiteral("qbutt/components"));
}

bool Qbutt::openBrowserUrl(const QString &url, const QString &choice)
{
    const QUrl address {url, QUrl::StrictMode};
    if (!address.isValid()
        || ((address.scheme() != QStringLiteral("https")) && (address.scheme() != QStringLiteral("http")))
        || !address.userInfo().isEmpty())
    {
        return false;
    }

    if (choice == QStringLiteral("auto"))
        return QDesktopServices::openUrl(address);

#if defined(Q_OS_WIN) || defined(Q_OS_LINUX)
    if ((choice == QStringLiteral("edge")) || (choice == QStringLiteral("chrome")))
    {
        const QString executable = browserPath(choice == QStringLiteral("edge"));
        return !executable.isEmpty() && QProcess::startDetached(executable, {address.toString(QUrl::FullyEncoded)});
    }
#endif
    return false;
}
