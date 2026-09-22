/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "update-registry.h"

#include <QCoreApplication>
#include <QDir>

#include "base/global.h"

UpdateRegistryFixture::UpdateRegistryFixture(const QString &applicationDirectory)
{
#ifdef Q_OS_WIN
    m_path = u"Software\\qbutt-update-acceptance-%1"_s.arg(QCoreApplication::applicationPid());
    if (RegCreateKeyExW(HKEY_CURRENT_USER, reinterpret_cast<LPCWSTR>(m_path.utf16()), 0, nullptr,
            REG_OPTION_VOLATILE, KEY_ALL_ACCESS, nullptr, &m_root, nullptr) != ERROR_SUCCESS)
        return;
    HKEY installation = nullptr;
    const auto key = L"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{64A54F85-79F8-43D3-9B5B-2336052C370E}_is1";
    if (RegCreateKeyExW(m_root, key, 0, nullptr, REG_OPTION_VOLATILE,
            KEY_ALL_ACCESS | KEY_WOW64_64KEY, nullptr, &installation, nullptr) != ERROR_SUCCESS)
        return;
    const QString path = QDir::toNativeSeparators(applicationDirectory);
    const LONG result = RegSetValueExW(installation, L"Inno Setup: App Path", 0, REG_SZ,
        reinterpret_cast<const BYTE *>(path.utf16()), static_cast<DWORD>((path.size() + 1) * sizeof(wchar_t)));
    RegCloseKey(installation);
    m_ready = (result == ERROR_SUCCESS) && (RegOverridePredefKey(HKEY_CURRENT_USER, m_root) == ERROR_SUCCESS);
#else
    Q_UNUSED(applicationDirectory)
#endif
}

UpdateRegistryFixture::~UpdateRegistryFixture()
{
#ifdef Q_OS_WIN
    if (m_ready)
        RegOverridePredefKey(HKEY_CURRENT_USER, nullptr);
    if (m_root)
    {
        RegCloseKey(m_root);
        RegDeleteTreeW(HKEY_CURRENT_USER, reinterpret_cast<LPCWSTR>(m_path.utf16()));
    }
#endif
}

bool UpdateRegistryFixture::isReady() const
{
    return m_ready;
}
