// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QString>

#ifdef Q_OS_WIN
#include <windows.h>
#endif

class BrowserRegistrationFixture
{
public:
    explicit BrowserRegistrationFixture(QString directory);
    ~BrowserRegistrationFixture();
    bool isReady() const;
    bool setRegistered(const QString &browser, bool registered);
    bool setInstalled(const QString &browser, bool installed);
    bool setDefault(const QString &browser);
    bool setQcatchInstalled();

private:
    QString m_directory;
    QString m_key;
    bool m_ready = false;
#ifdef Q_OS_WIN
    HKEY m_user = nullptr;
    HKEY m_machine = nullptr;
#endif
};
