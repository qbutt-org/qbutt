// SPDX-License-Identifier: GPL-2.0-or-later
#include "browser-registration.h"

#include <utility>

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QUuid>

namespace
{
#ifdef Q_OS_WIN
    bool setString(const QString &key, const QString &name, const QString &value)
    {
        HKEY handle = nullptr;
        if (RegCreateKeyExW(HKEY_CURRENT_USER, reinterpret_cast<LPCWSTR>(key.utf16()), 0, nullptr, 0,
                KEY_SET_VALUE, nullptr, &handle, nullptr) != ERROR_SUCCESS)
        {
            return false;
        }
        const auto result = RegSetValueExW(handle, name.isEmpty() ? nullptr : reinterpret_cast<LPCWSTR>(name.utf16()),
            0, REG_SZ, reinterpret_cast<const BYTE *>(value.utf16()),
            static_cast<DWORD>((value.size() + 1) * sizeof(wchar_t)));
        RegCloseKey(handle);
        return result == ERROR_SUCCESS;
    }
#endif
}

BrowserRegistrationFixture::BrowserRegistrationFixture(QString directory)
    : m_directory {std::move(directory)}
{
#ifdef Q_OS_WIN
    m_key = QStringLiteral("Software\\qbutt-acceptance\\browser-") + QUuid::createUuid().toString(QUuid::WithoutBraces);
    const QString user = m_key + QStringLiteral("\\User");
    const QString machine = m_key + QStringLiteral("\\Machine");
    if ((RegCreateKeyExW(HKEY_CURRENT_USER, reinterpret_cast<LPCWSTR>(user.utf16()), 0, nullptr, 0,
            KEY_ALL_ACCESS, nullptr, &m_user, nullptr) != ERROR_SUCCESS)
        || (RegCreateKeyExW(HKEY_CURRENT_USER, reinterpret_cast<LPCWSTR>(machine.utf16()), 0, nullptr, 0,
            KEY_ALL_ACCESS, nullptr, &m_machine, nullptr) != ERROR_SUCCESS))
    {
        return;
    }
    if (RegOverridePredefKey(HKEY_CURRENT_USER, m_user) != ERROR_SUCCESS)
        return;
    if (RegOverridePredefKey(HKEY_LOCAL_MACHINE, m_machine) != ERROR_SUCCESS)
    {
        RegOverridePredefKey(HKEY_CURRENT_USER, nullptr);
        return;
    }
    m_ready = true;
#endif
}

BrowserRegistrationFixture::~BrowserRegistrationFixture()
{
#ifdef Q_OS_WIN
    if (m_ready)
    {
        RegOverridePredefKey(HKEY_CURRENT_USER, nullptr);
        RegOverridePredefKey(HKEY_LOCAL_MACHINE, nullptr);
    }
    if (m_user)
        RegCloseKey(m_user);
    if (m_machine)
        RegCloseKey(m_machine);
    if (m_key.startsWith(QStringLiteral("Software\\qbutt-acceptance\\browser-")) && m_key.size() > 60)
        RegDeleteTreeW(HKEY_CURRENT_USER, reinterpret_cast<LPCWSTR>(m_key.utf16()));
#endif
}

bool BrowserRegistrationFixture::isReady() const
{
    return m_ready;
}

bool BrowserRegistrationFixture::setRegistered(const QString &browser, const bool registered)
{
#ifdef Q_OS_WIN
    if (!m_ready || ((browser != QStringLiteral("edge")) && (browser != QStringLiteral("chrome"))))
        return false;
    const QString key = QStringLiteral("Software\\")
        + (browser == QStringLiteral("edge") ? QStringLiteral("Microsoft\\Edge") : QStringLiteral("Google\\Chrome"))
        + QStringLiteral("\\NativeMessagingHosts\\org.qbutt.qcatch");
    if (!registered)
    {
        const auto result = RegDeleteTreeW(HKEY_CURRENT_USER, reinterpret_cast<LPCWSTR>(key.utf16()));
        return (result == ERROR_SUCCESS) || (result == ERROR_FILE_NOT_FOUND);
    }
    const QString executable = qEnvironmentVariable("QBUTT_QCATCH_EXE");
    if (!QFileInfo(executable).isFile())
        return false;
    const QString path = QDir(m_directory).filePath(QStringLiteral("native-host.json"));
    QFile file(path);
    const QJsonDocument manifest(QJsonObject {
        {QStringLiteral("name"), QStringLiteral("org.qbutt.qcatch")},
        {QStringLiteral("type"), QStringLiteral("stdio")},
        {QStringLiteral("path"), executable},
        {QStringLiteral("allowed_origins"), QJsonArray {QStringLiteral("chrome-extension://jnmnnglmejlbjnjmmjampnhfncpcbffh/")}}
    });
    if (!file.open(QIODevice::WriteOnly) || (file.write(manifest.toJson()) < 0))
        return false;
    file.close();
    HKEY handle = nullptr;
    if (RegCreateKeyExW(HKEY_CURRENT_USER, reinterpret_cast<LPCWSTR>(key.utf16()), 0, nullptr, 0,
            KEY_SET_VALUE, nullptr, &handle, nullptr) != ERROR_SUCCESS)
    {
        return false;
    }
    const auto result = RegSetValueExW(handle, nullptr, 0, REG_SZ,
        reinterpret_cast<const BYTE *>(path.utf16()), static_cast<DWORD>((path.size() + 1) * sizeof(wchar_t)));
    RegCloseKey(handle);
    return result == ERROR_SUCCESS;
#else
    Q_UNUSED(browser)
    Q_UNUSED(registered)
    return false;
#endif
}

bool BrowserRegistrationFixture::setInstalled(const QString &browser, const bool installed)
{
#ifdef Q_OS_WIN
    if (!m_ready || (browser != QStringLiteral("edge") && browser != QStringLiteral("chrome")))
        return false;
    const QString executable = qEnvironmentVariable("QBUTT_QCATCH_SETUP_FIXTURE_EXE");
    if (installed && !QFileInfo(executable).isFile())
        return false;
    const QString name = browser == QStringLiteral("edge") ? QStringLiteral("msedge.exe") : QStringLiteral("chrome.exe");
    const QString key = QStringLiteral("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\") + name;
    if (!installed)
    {
        const auto result = RegDeleteTreeW(HKEY_CURRENT_USER, reinterpret_cast<LPCWSTR>(key.utf16()));
        return result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND;
    }
    return setString(key, {}, executable);
#else
    Q_UNUSED(browser)
    Q_UNUSED(installed)
    return false;
#endif
}

bool BrowserRegistrationFixture::setDefault(const QString &browser)
{
#ifdef Q_OS_WIN
    if (!m_ready || (browser != QStringLiteral("edge") && browser != QStringLiteral("chrome")))
        return false;
    return setString(QStringLiteral("Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\https\\UserChoice"),
        QStringLiteral("ProgId"), browser == QStringLiteral("edge") ? QStringLiteral("MSEdgeHTM") : QStringLiteral("ChromeHTML"));
#else
    Q_UNUSED(browser)
    return false;
#endif
}

bool BrowserRegistrationFixture::setQcatchInstalled()
{
#ifdef Q_OS_WIN
    const QFileInfo executable(qEnvironmentVariable("QBUTT_QCATCH_SETUP_FIXTURE_EXE"));
    return m_ready && executable.isFile() && executable.fileName() == QStringLiteral("qcatch.exe")
        && setString(QStringLiteral("Software\\qbutt-org\\qcatch"), QStringLiteral("InstallDir"),
            executable.absolutePath());
#else
    return false;
#endif
}
