/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QString>

#ifdef Q_OS_WIN
#include <windows.h>
#endif

// The installation marker is visible only to this process. The real uninstall
// key is never modified, even when an actual qbutt installation already exists.
class UpdateRegistryFixture
{
    Q_DISABLE_COPY_MOVE(UpdateRegistryFixture)

public:
    explicit UpdateRegistryFixture(const QString &applicationDirectory);
    ~UpdateRegistryFixture();
    bool isReady() const;

private:
#ifdef Q_OS_WIN
    HKEY m_root = nullptr;
    QString m_path;
#endif
    bool m_ready = false;
};
