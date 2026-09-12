/*
 * qbutt, a native Qt BitTorrent client.
 * Copyright (C) 2026 qbutt contributors
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the
 * Free Software Foundation; either version 2 of the License, or (at your
 * option) any later version.
 */

#pragma once

#include <QDialog>

class QCheckBox;
class QLabel;
class QProgressBar;
class QPushButton;
class QTreeWidget;

namespace BitTorrent
{
    struct RepairAnalysis;
    class RepairService;
    class Torrent;
}

class RepairDialog final : public QDialog
{
    Q_OBJECT
    Q_DISABLE_COPY_MOVE(RepairDialog)

public:
    RepairDialog(QWidget *parent, BitTorrent::Torrent *torrent);

private:
    void showAnalysis(const BitTorrent::RepairAnalysis &analysis);
    void showFailure(const QString &message);

    BitTorrent::RepairService *m_service = nullptr;
    QLabel *m_status = nullptr;
    QProgressBar *m_progress = nullptr;
    QTreeWidget *m_files = nullptr;
    QCheckBox *m_consent = nullptr;
    QPushButton *m_apply = nullptr;
};
