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

namespace
{
#ifdef Q_OS_WIN
    QString browserPath(const bool edge)
    {
        const QString relative = edge ? QStringLiteral("Microsoft/Edge/Application/msedge.exe")
            : QStringLiteral("Google/Chrome/Application/chrome.exe");
        for (const QString &root : {qEnvironmentVariable("LOCALAPPDATA"), qEnvironmentVariable("ProgramFiles"),
            qEnvironmentVariable("ProgramFiles(x86)")})
        {
            if (root.isEmpty())
                continue;
            const QString path = QDir(root).filePath(relative);
            if (QFileInfo(path).isFile())
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
            if (path.isEmpty() || !QFileInfo::exists(path))
                continue;
            QFile manifest(path);
            if (!manifest.open(QIODevice::ReadOnly))
            {
                state = Qbutt::NativeHostState::Unknown;
                continue;
            }
            if (manifest.size() > 64 * 1024)
                continue;
            const QJsonObject value = QJsonDocument::fromJson(manifest.readAll()).object();
            const QString executable = value.value(QStringLiteral("path")).toString();
            const QJsonArray origins = value.value(QStringLiteral("allowed_origins")).toArray();
            if ((value.value(QStringLiteral("name")).toString() == QStringLiteral("org.qbutt.qcatch"))
                && (value.value(QStringLiteral("type")).toString() == QStringLiteral("stdio"))
                && QFileInfo(executable).isFile()
                && origins.contains(QStringLiteral("chrome-extension://jnmnnglmejlbjnjmmjampnhfncpcbffh/")))
            {
                return Qbutt::NativeHostState::Registered;
            }
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
#endif
    return browsers;
}

QString Qbutt::browserDiscoveryPath()
{
    const QString override = qEnvironmentVariable("QBUTT_BROWSER_BRIDGE_DISCOVERY");
    if (!override.isEmpty())
        return override;

    const QDir localData {QStandardPaths::writableLocation(QStandardPaths::GenericDataLocation)};
    return localData.filePath(QStringLiteral("qbutt/content/browser-bridge.json"));
}

bool Qbutt::openBrowserUrl(const QString &url, const QString &choice)
{
    const QUrl address {url, QUrl::StrictMode};
    if (!address.isValid() || (address.scheme() != QStringLiteral("https"))
        || !address.userInfo().isEmpty())
    {
        return false;
    }

    if (choice == QStringLiteral("auto"))
        return QDesktopServices::openUrl(address);

#ifdef Q_OS_WIN
    if ((choice == QStringLiteral("edge")) || (choice == QStringLiteral("chrome")))
    {
        const QString executable = browserPath(choice == QStringLiteral("edge"));
        return !executable.isEmpty() && QProcess::startDetached(executable, {address.toString(QUrl::FullyEncoded)});
    }
#endif
    return false;
}
