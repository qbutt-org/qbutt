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

#include <atomic>
#include <memory>
#include <optional>

#include <QDialog>
#include <QFutureWatcher>
#include <QMap>
#include <QThreadPool>

#include "base/bittorrent/repairplan.h"
#include "base/bittorrent/torrentdescriptor.h"

class QCheckBox;
class QComboBox;
class QLabel;
class QPlainTextEdit;
class QProgressBar;
class QPushButton;
class QTableWidget;

class FileSystemPathLineEdit;

class RepairPreviewDialog final : public QDialog
{
    Q_OBJECT
    Q_DISABLE_COPY_MOVE(RepairPreviewDialog)

public:
    explicit RepairPreviewDialog(QWidget *parent = nullptr);
    ~RepairPreviewDialog() override;

    void reject() override;

private:
    enum class Operation
    {
        Idle,
        Preview,
        Adding
    };

    void clearPreview();
    void preview();
    void chooseSource();
    void startRepair();
    void showPreview();
    void updateControls();
    QStringList sourceRoots() const;

    Operation m_operation = Operation::Idle;
    std::optional<BitTorrent::TorrentDescriptor> m_descriptor;
    std::optional<BitTorrent::RepairPlan> m_plan;
    QMap<int, QString> m_explicitMappings;
    std::shared_ptr<std::atomic_bool> m_cancelled;
    bool m_closePending = false;
    QThreadPool m_worker;
    QFutureWatcher<BitTorrent::RepairPlan> m_previewWatcher;
    QMetaObject::Connection m_torrentAddedConnection;
    QMetaObject::Connection m_addTorrentFailedConnection;
    FileSystemPathLineEdit *m_torrentFile = nullptr;
    FileSystemPathLineEdit *m_destination = nullptr;
    QPlainTextEdit *m_sourceDirectories = nullptr;
    QComboBox *m_mode = nullptr;
    QLabel *m_status = nullptr;
    QLabel *m_candidateBytes = nullptr;
    QLabel *m_verifiedBytes = nullptr;
    QLabel *m_networkBytes = nullptr;
    QLabel *m_temporaryBytes = nullptr;
    QLabel *m_changedFiles = nullptr;
    QLabel *m_oversizedFiles = nullptr;
    QProgressBar *m_progress = nullptr;
    QTableWidget *m_files = nullptr;
    QCheckBox *m_reviewed = nullptr;
    QPushButton *m_addSourceDirectory = nullptr;
    QPushButton *m_chooseSource = nullptr;
    QPushButton *m_previewButton = nullptr;
    QPushButton *m_applyButton = nullptr;
    QPushButton *m_closeButton = nullptr;
};
