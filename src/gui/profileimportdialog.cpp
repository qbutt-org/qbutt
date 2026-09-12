/*
 * qbutt, a native Qt BitTorrent client.
 * Copyright (C) 2026 qbutt contributors
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the
 * Free Software Foundation; either version 2 of the License, or (at your
 * option) any later version.
 */

#include "profileimportdialog.h"

#include <utility>

#include <libtorrent/torrent_info.hpp>

#include <QCheckBox>
#include <QDialogButtonBox>
#include <QFormLayout>
#include <QHeaderView>
#include <QLabel>
#include <QMessageBox>
#include <QProgressBar>
#include <QPushButton>
#include <QSignalBlocker>
#include <QTableWidget>
#include <QTextDocument>
#include <QTreeWidget>
#include <QVBoxLayout>
#include <QtConcurrentRun>

#include "base/global.h"
#include "fspathedit.h"

ProfileImportDialog::ProfileImportDialog(QWidget *parent)
    : QDialog {parent}
    , m_sources {new QWidget {this}}
    , m_settingsFile {new FileSystemPathLineEdit {m_sources}}
    , m_dataDirectory {new FileSystemPathLineEdit {m_sources}}
    , m_sourceBase {new FileSystemPathLineEdit {m_sources}}
    , m_status {new QLabel {this}}
    , m_progress {new QProgressBar {this}}
    , m_settings {new QTreeWidget {this}}
    , m_torrents {new QTableWidget {this}}
    , m_ownership {new QCheckBox {tr("The source client is closed and no longer manages the selected data."), this}}
{
    setObjectName(u"ProfileImportDialog"_s);
    setWindowTitle(tr("Import profile"));
    setWindowModality(Qt::WindowModal);
    resize(960, 720);

    auto *layout = new QVBoxLayout {this};
    auto *introduction = new QLabel {tr("Import selected qBittorrent settings and torrents into qbutt on its next start. "
        "The source profile is read-only. Payload files are not moved or copied, and all imported torrents start stopped."), this};
    introduction->setWordWrap(true);
    layout->addWidget(introduction);

    m_settingsFile->setObjectName(u"profileSettingsFile"_s);
    m_settingsFile->setMode(FileSystemPathEdit::Mode::FileOpen);
    m_settingsFile->setDialogCaption(tr("Choose qBittorrent settings"));
    m_settingsFile->setFileNameFilter(tr("Settings files (*.ini *.conf);;All files (*)"));
    m_dataDirectory->setObjectName(u"profileDataDirectory"_s);
    m_dataDirectory->setMode(FileSystemPathEdit::Mode::DirectoryOpen);
    m_dataDirectory->setDialogCaption(tr("Choose the data directory containing BT_backup or torrents.db"));
    m_sourceBase->setObjectName(u"profileSourceBase"_s);
    m_sourceBase->setMode(FileSystemPathEdit::Mode::DirectoryOpen);
    m_sourceBase->setDialogCaption(tr("Choose the source portable profile base"));
    auto *form = new QFormLayout {m_sources};
    form->setContentsMargins(0, 0, 0, 0);
    form->addRow(tr("Settings file:"), m_settingsFile);
    form->addRow(tr("Data directory:"), m_dataDirectory);
    form->addRow(tr("Portable profile base (optional):"), m_sourceBase);
    layout->addWidget(m_sources);
    auto *sourceHint = new QLabel {tr("A portable profile base is needed only when the source uses relative paths. "
        "Otherwise, source and destination paths must be absolute."), this};
    sourceHint->setWordWrap(true);
    layout->addWidget(sourceHint);

    m_status->setObjectName(u"profileImportStatus"_s);
    m_status->setTextFormat(Qt::PlainText);
    m_status->setTextInteractionFlags(Qt::TextSelectableByMouse);
    m_status->setWordWrap(true);
    m_status->setText(tr("Choose the source files, then preview the import."));
    layout->addWidget(m_status);
    m_progress->setRange(0, 0);
    m_progress->setTextVisible(false);
    layout->addWidget(m_progress);

    m_settings->setObjectName(u"profileImportSettings"_s);
    m_settings->setHeaderLabels({tr("Settings"), tr("Value to import")});
    m_settings->setEditTriggers(QAbstractItemView::NoEditTriggers);
    m_settings->header()->setSectionResizeMode(QHeaderView::ResizeToContents);
    m_settings->header()->setStretchLastSection(true);
    m_settings->setMaximumHeight(150);
    layout->addWidget(m_settings);
    auto *policySummary = new QLabel {tr("Only the settings shown above are imported. Completion policies remain in preview until reviewed in qbutt. "
        "Expand Skipped settings to review the excluded names."), this};
    policySummary->setWordWrap(true);
    layout->addWidget(policySummary);

    m_torrents->setObjectName(u"profileImportTorrents"_s);
    m_torrents->setColumnCount(4);
    m_torrents->setHorizontalHeaderLabels({tr("Import"), tr("Torrent"), tr("Source payload location"), tr("Destination root")});
    m_torrents->verticalHeader()->hide();
    m_torrents->setAlternatingRowColors(true);
    m_torrents->setSelectionBehavior(QAbstractItemView::SelectRows);
    m_torrents->horizontalHeader()->setSectionResizeMode(0, QHeaderView::ResizeToContents);
    for (int column = 1; column < 4; ++column)
        m_torrents->horizontalHeader()->setSectionResizeMode(column, QHeaderView::Stretch);
    layout->addWidget(m_torrents, 1);
    auto *mappingHint = new QLabel {tr("Edit a destination root to point to the existing payload on this computer. "
        "Unchecked torrents will not be imported."), this};
    mappingHint->setWordWrap(true);
    layout->addWidget(mappingHint);
    m_ownership->setObjectName(u"profileImportOwnership"_s);
    layout->addWidget(m_ownership);

    auto *buttons = new QDialogButtonBox {QDialogButtonBox::Close, this};
    m_previewButton = buttons->addButton(tr("Preview"), QDialogButtonBox::ActionRole);
    m_previewButton->setObjectName(u"profileImportPreview"_s);
    m_importButton = buttons->addButton(tr("Import on next start"), QDialogButtonBox::ActionRole);
    m_importButton->setObjectName(u"profileImportApply"_s);
    m_importButton->setAutoDefault(false);
    m_closeButton = buttons->button(QDialogButtonBox::Close);
    layout->addWidget(buttons);

    for (FileSystemPathLineEdit *field : {m_settingsFile, m_dataDirectory, m_sourceBase})
        connect(field, &FileSystemPathEdit::selectedPathChanged, this, &ProfileImportDialog::clearPreview);
    connect(buttons, &QDialogButtonBox::rejected, this, &ProfileImportDialog::reject);
    connect(m_previewButton, &QPushButton::clicked, this, &ProfileImportDialog::preview);
    connect(m_importButton, &QPushButton::clicked, this, &ProfileImportDialog::prepareImport);
    connect(m_ownership, &QCheckBox::toggled, this, &ProfileImportDialog::updateControls);
    connect(m_torrents, &QTableWidget::itemChanged, this, &ProfileImportDialog::updateControls);
    connect(&m_previewWatcher, &QFutureWatcherBase::finished, this, [this]
    {
        m_operation = Operation::Idle;
        auto result = m_previewWatcher.future().takeResult();
        if (result)
        {
            m_preview = std::move(*result);
            showPreview();
        }
        else
        {
            m_status->setText(tr("Cannot preview this profile: %1").arg(result.error()));
        }
        updateControls();
    });
    connect(&m_importWatcher, &QFutureWatcherBase::finished, this, [this]
    {
        m_operation = Operation::Idle;
        const auto result = m_importWatcher.future().takeResult();
        if (!result)
        {
            m_status->setText(tr("Import was not prepared: %1").arg(result.error()));
            updateControls();
            return;
        }
        auto *notice = new QMessageBox {QMessageBox::Information, tr("Import prepared")
            , tr("Restart qbutt to finish importing the selected settings and stopped torrents. "
                "Keep the source client closed; it must no longer manage the selected data.")
            , QMessageBox::Ok, this};
        m_progress->hide();
        notice->setAttribute(Qt::WA_DeleteOnClose);
        connect(notice, &QMessageBox::finished, this, &QDialog::accept);
        notice->open();
    });
    updateControls();
}

void ProfileImportDialog::preview()
{
    clearPreview();
    m_operation = Operation::Preview;
    m_status->setText(tr("Reading the source profile. No files are changed."));
    updateControls();
    m_previewWatcher.setFuture(QtConcurrent::run(&m_worker, [settings = m_settingsFile->selectedPath()
        , data = m_dataDirectory->selectedPath(), sourceBase = m_sourceBase->selectedPath()]
    {
        return ProfileImport::preview(settings, data, sourceBase);
    }));
}

void ProfileImportDialog::prepareImport()
{
    if (!m_preview || !m_ownership->isChecked())
        return;
    ProfileImportPreview selected = *m_preview;
    for (int row = 0; row < selected.torrents.size(); ++row)
    {
        selected.torrents[row].selected = (m_torrents->item(row, 0)->checkState() == Qt::Checked);
        selected.torrents[row].destinationPath = Path {m_torrents->item(row, 3)->text()};
    }
    m_operation = Operation::Import;
    m_status->setText(tr("Preparing the import for qbutt's next start. Wait for this operation to finish before closing."));
    updateControls();
    m_importWatcher.setFuture(QtConcurrent::run(&m_worker, [selected = std::move(selected)]() mutable
    {
        return ProfileImport::prepare(std::move(selected), true);
    }));
}

void ProfileImportDialog::clearPreview()
{
    m_preview.reset();
    m_settings->clear();
    m_torrents->setRowCount(0);
    m_ownership->setChecked(false);
    m_status->setText(tr("Choose the source files, then preview the import."));
    updateControls();
}

void ProfileImportDialog::showPreview()
{
    auto *imported = new QTreeWidgetItem {m_settings, {tr("Settings to import (%L1)").arg(m_preview->settings.size())}};
    for (auto it = m_preview->settings.cbegin(); it != m_preview->settings.cend(); ++it)
        new QTreeWidgetItem {imported, {it.key(), it.value().toString()}};
    imported->setExpanded(true);
    auto *skipped = new QTreeWidgetItem {m_settings, {tr("Skipped settings (%L1)").arg(m_preview->skippedSettings.size())}};
    for (const QString &key : m_preview->skippedSettings)
        new QTreeWidgetItem {skipped, {key}};
    const QSignalBlocker blocker {m_torrents};
    m_torrents->setRowCount(static_cast<int>(m_preview->torrents.size()));
    for (int row = 0; row < m_preview->torrents.size(); ++row)
    {
        const ProfileImportTorrent &torrent = m_preview->torrents[row];
        auto *selection = new QTableWidgetItem;
        selection->setFlags(Qt::ItemIsEnabled | Qt::ItemIsSelectable | Qt::ItemIsUserCheckable);
        selection->setCheckState(torrent.selected ? Qt::Checked : Qt::Unchecked);
        m_torrents->setItem(row, 0, selection);
        const auto &native = torrent.params.ltAddTorrentParams;
        QString name = torrent.params.name;
        if (name.isEmpty())
            name = QString::fromStdString(native.ti->name());
        if (name.isEmpty())
            name = torrent.id.toString();
        const lt::file_storage &files = native.ti->files();
        QStringList mappedFiles;
        int fileCount = 0;
        for (const lt::file_index_t index : files.file_range())
        {
            if (files.pad_file_at(index))
                continue;
            ++fileCount;
            if (mappedFiles.size() >= 100)
                continue;
            const QString original = QString::fromStdString(files.file_path(index));
            const auto renamed = native.renamed_files.find(index);
            const QString mapped = (renamed == native.renamed_files.end()) ? original : QString::fromStdString(renamed->second);
            mappedFiles.append((mapped == original) ? original : tr("%1 → %2").arg(original, mapped));
        }
        QString details = tr("Relative file paths (%L1):\n%2").arg(fileCount).arg(mappedFiles.join(u'\n'));
        if (fileCount > mappedFiles.size())
            details += tr("\nShowing the first %L1 files.").arg(mappedFiles.size());
        for (const auto &[column, text] : {std::pair {1, name}, std::pair {2, torrent.sourcePath.toString()}
            , std::pair {3, torrent.destinationPath.toString()}})
        {
            auto *item = new QTableWidgetItem {text};
            item->setFlags(Qt::ItemIsEnabled | Qt::ItemIsSelectable | ((column == 3) ? Qt::ItemIsEditable : Qt::NoItemFlags));
            const QString tooltip = (column == 1) ? torrent.id.toString()
                : (column == 2) ? torrent.sourcePath.toString() + u"\n\n" + details : details;
            item->setToolTip(Qt::convertFromPlainText(tooltip));
            m_torrents->setItem(row, column, item);
        }
    }
    m_status->setText(tr("Preview ready: %L1 safe settings and %L2 torrents. Review the destination roots before importing.")
        .arg(m_preview->settings.size()).arg(m_preview->torrents.size()));
}

void ProfileImportDialog::updateControls()
{
    const bool idle = (m_operation == Operation::Idle);
    m_sources->setEnabled(idle);
    m_previewButton->setEnabled(idle && m_settingsFile->selectedPath().isAbsolute() && m_dataDirectory->selectedPath().isAbsolute()
        && (m_sourceBase->selectedPath().isEmpty() || m_sourceBase->selectedPath().isAbsolute()));
    m_torrents->setEnabled(idle && m_preview.has_value());
    m_ownership->setEnabled(idle && m_preview.has_value());
    bool hasSelection = false;
    bool validPaths = true;
    for (int row = 0; row < m_torrents->rowCount(); ++row)
    {
        if (const QTableWidgetItem *item = m_torrents->item(row, 0); item && (item->checkState() == Qt::Checked))
        {
            hasSelection = true;
            const QTableWidgetItem *destination = m_torrents->item(row, 3);
            validPaths &= destination && Path {destination->text()}.isAbsolute();
        }
    }
    m_importButton->setEnabled(idle && m_ownership->isChecked() && hasSelection && validPaths);
    m_closeButton->setEnabled(m_operation != Operation::Import);
    m_progress->setVisible(!idle);
}

void ProfileImportDialog::reject()
{
    if (m_operation != Operation::Import)
        QDialog::reject();
}
