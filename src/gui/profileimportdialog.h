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

#include <optional>

#include <QDialog>
#include <QFutureWatcher>
#include <QThreadPool>

#include "base/profileimport.h"

class QCheckBox;
class QLabel;
class QProgressBar;
class QPushButton;
class QTableWidget;
class QTreeWidget;

class FileSystemPathLineEdit;

class ProfileImportDialog final : public QDialog
{
    Q_OBJECT
    Q_DISABLE_COPY_MOVE(ProfileImportDialog)

public:
    explicit ProfileImportDialog(QWidget *parent = nullptr);

    void reject() override;

private:
    enum class Operation
    {
        Idle,
        Preview,
        Import
    };

    void preview();
    void prepareImport();
    void clearPreview();
    void showPreview();
    void updateControls();

    Operation m_operation = Operation::Idle;
    std::optional<ProfileImportPreview> m_preview;
    // Destroying the dialog drains jobs before application-owned profile state.
    QThreadPool m_worker;
    QFutureWatcher<nonstd::expected<ProfileImportPreview, QString>> m_previewWatcher;
    QFutureWatcher<nonstd::expected<void, QString>> m_importWatcher;
    QWidget *m_sources = nullptr;
    FileSystemPathLineEdit *m_settingsFile = nullptr;
    FileSystemPathLineEdit *m_dataDirectory = nullptr;
    FileSystemPathLineEdit *m_sourceBase = nullptr;
    QLabel *m_status = nullptr;
    QProgressBar *m_progress = nullptr;
    QTreeWidget *m_settings = nullptr;
    QTableWidget *m_torrents = nullptr;
    QCheckBox *m_ownership = nullptr;
    QPushButton *m_previewButton = nullptr;
    QPushButton *m_importButton = nullptr;
    QPushButton *m_closeButton = nullptr;
};
