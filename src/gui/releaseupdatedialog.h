/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QDialog>

#include "base/releaseupdater.h"

class QLabel;
class QProgressBar;
class QPushButton;

class ReleaseUpdateDialog final : public QDialog
{
    Q_OBJECT
    Q_DISABLE_COPY_MOVE(ReleaseUpdateDialog)

public:
    explicit ReleaseUpdateDialog(QWidget *parent = nullptr);

private:
    void refresh();

    ReleaseUpdater m_updater;
    QLabel *m_status;
    QProgressBar *m_progress;
    QPushButton *m_check;
    QPushButton *m_download;
    QPushButton *m_cancel;
    QPushButton *m_openFolder;
};
