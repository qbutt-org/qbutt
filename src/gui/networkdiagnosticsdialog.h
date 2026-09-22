/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QDialog>
#include <QElapsedTimer>
#include <QJsonObject>
#include <QList>
#include <QPointer>

class QLabel;
class QListWidget;
class QPushButton;
class QTableWidget;
class QTimer;

namespace BitTorrent
{
    class Torrent;
    struct TorrentDiagnosticStatus;
    struct TorrentPeerDiagnosticStatus;
}

class NetworkDiagnosticsDialog final : public QDialog
{
    Q_OBJECT
    Q_DISABLE_COPY_MOVE(NetworkDiagnosticsDialog)

public:
    explicit NetworkDiagnosticsDialog(QWidget *parent, BitTorrent::Torrent *torrent);

    BitTorrent::Torrent *torrent() const;

private slots:
    void refreshNow();
    void exportDiagnostics();

private:
    QJsonObject currentSnapshot() const;
    QJsonObject buildSnapshot(const BitTorrent::TorrentDiagnosticStatus &status,
        const BitTorrent::TorrentPeerDiagnosticStatus &peers) const;
    QJsonObject anonymizedExport() const;
    void prune();
    void retain(QJsonObject snapshot);
    void render(const QJsonObject &snapshot);

    QPointer<BitTorrent::Torrent> m_torrent;
    QTimer *m_timer = nullptr;
    QLabel *m_torrentLabel = nullptr;
    QLabel *m_transfer = nullptr;
    QTableWidget *m_summary = nullptr;
    QTableWidget *m_paths = nullptr;
    QListWidget *m_reasons = nullptr;
    QPushButton *m_refresh = nullptr;
    QPushButton *m_export = nullptr;
    QLabel *m_status = nullptr;
    QList<QJsonObject> m_samples;
    QElapsedTimer m_clock;
    bool m_refreshing = false;
};
