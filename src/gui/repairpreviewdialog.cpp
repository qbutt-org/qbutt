/*
 * qbutt, a native Qt BitTorrent client.
 * Copyright (C) 2026 qbutt contributors
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the
 * Free Software Foundation; either version 2 of the License, or (at your
 * option) any later version.
 */

#include "repairpreviewdialog.h"

#include <tuple>

#include <libtorrent/file_storage.hpp>

#include <QCheckBox>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QFormLayout>
#include <QHeaderView>
#include <QLabel>
#include <QLocale>
#include <QPlainTextEdit>
#include <QProgressBar>
#include <QPushButton>
#include <QSignalBlocker>
#include <QTableWidget>
#include <QToolButton>
#include <QVBoxLayout>
#include <QtConcurrentRun>

#include "base/bittorrent/addtorrenterror.h"
#include "base/bittorrent/addtorrentparams.h"
#include "base/bittorrent/downloadpriority.h"
#include "base/bittorrent/infohash.h"
#include "base/bittorrent/session.h"
#include "base/bittorrent/torrent.h"
#include "base/bittorrent/torrentcontentlayout.h"
#include "base/global.h"
#include "base/path.h"
#include "fspathedit.h"
#include "repairdialog.h"

using namespace BitTorrent;

RepairPreviewDialog::RepairPreviewDialog(QWidget *parent)
    : QDialog {parent}
    , m_torrentFile {new FileSystemPathLineEdit {this}}
    , m_destination {new FileSystemPathLineEdit {this}}
    , m_sourceDirectories {new QPlainTextEdit {this}}
    , m_mode {new QComboBox {this}}
    , m_status {new QLabel {this}}
    , m_candidateBytes {new QLabel {this}}
    , m_verifiedBytes {new QLabel {this}}
    , m_networkBytes {new QLabel {this}}
    , m_temporaryBytes {new QLabel {this}}
    , m_changedFiles {new QLabel {this}}
    , m_oversizedFiles {new QLabel {this}}
    , m_progress {new QProgressBar {this}}
    , m_files {new QTableWidget {this}}
    , m_reviewed {new QCheckBox {tr("Allow adding the torrent and creating any missing empty files."), this}}
{
    setObjectName(u"RepairPreviewDialog"_s);
    setWindowTitle(tr("Smart repair from torrent file"));
    setWindowModality(Qt::WindowModal);
    resize(760, 0);
    m_worker.setMaxThreadCount(1);

    auto *layout = new QVBoxLayout {this};
    m_torrentFile->setObjectName(u"repairPreviewTorrent"_s);
    m_torrentFile->setMode(FileSystemPathEdit::Mode::FileOpen);
    m_torrentFile->setDialogCaption(tr("Choose target torrent"));
    m_torrentFile->setFileNameFilter(tr("Torrent files (*.torrent)"));
    m_destination->setObjectName(u"repairPreviewDestination"_s);
    m_destination->setMode(FileSystemPathEdit::Mode::DirectoryOpen);
    m_destination->setDialogCaption(tr("Choose the folder containing the torrent's files"));
    m_destination->setToolTip(tr("The parent folder for the paths listed in the torrent."));
    m_sourceDirectories->setObjectName(u"repairPreviewRoots"_s);
    m_sourceDirectories->setPlaceholderText(tr("Other folders to search, one per line"));
    m_sourceDirectories->setMaximumHeight(70);
    m_mode->setObjectName(u"repairPreviewMode"_s);
    m_mode->addItems({tr("Repair a separate copy first"), tr("Change original files, without a backup")});
    auto *sources = new QFormLayout;
    sources->setRowWrapPolicy(QFormLayout::DontWrapRows);
    sources->addRow(tr("Torrent:"), m_torrentFile);
    sources->addRow(tr("Folder:"), m_destination);
    layout->addLayout(sources);

    m_status->setObjectName(u"repairPreviewStatus"_s);
    m_status->setTextFormat(Qt::PlainText);
    m_status->setTextInteractionFlags(Qt::TextSelectableByMouse);
    m_status->setWordWrap(true);
    m_status->setText(tr("Scanning does not change your files."));
    layout->addWidget(m_status);
    m_progress->setObjectName(u"repairPreviewProgress"_s);
    m_progress->setRange(0, 0);
    m_progress->setTextVisible(false);
    layout->addWidget(m_progress);

    m_results = new QWidget {this};
    m_results->setObjectName(u"repairPreviewResults"_s);
    auto *results = new QFormLayout {m_results};
    results->setContentsMargins(0, 0, 0, 0);
    layout->addWidget(m_results);

    auto *detailsToggle = new QToolButton {this};
    detailsToggle->setObjectName(u"repairPreviewDetails"_s);
    detailsToggle->setText(tr("Files and options"));
    detailsToggle->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
    detailsToggle->setArrowType(Qt::RightArrow);
    detailsToggle->setCheckable(true);
    layout->addWidget(detailsToggle, 0, Qt::AlignLeft);
    auto *details = new QWidget {this};
    auto *detailsLayout = new QVBoxLayout {details};
    detailsLayout->setContentsMargins(0, 0, 0, 0);
    auto *options = new QFormLayout;
    options->addRow(tr("Repair method:"), m_mode);
    options->addRow(tr("Other folders:"), m_sourceDirectories);
    m_addSourceDirectory = new QPushButton {tr("Add folder…"), this};
    m_addSourceDirectory->setObjectName(u"repairPreviewAddRoot"_s);
    options->addRow(QString {}, m_addSourceDirectory);
    detailsLayout->addLayout(options);
    m_detailsSummary = new QWidget {details};
    auto *detailsSummary = new QFormLayout {m_detailsSummary};
    detailsSummary->setContentsMargins(0, 0, 0, 0);
    for (auto [label, name, value, target] : {
        std::tuple {tr("Reusable:"), u"repairPreviewVerified"_s, m_verifiedBytes, results},
        std::tuple {tr("To download:"), u"repairPreviewNetwork"_s, m_networkBytes, results},
        std::tuple {tr("Data found:"), u"repairPreviewCandidates"_s, m_candidateBytes, detailsSummary},
        std::tuple {tr("Extra space:"), u"repairPreviewTemporary"_s, m_temporaryBytes, detailsSummary},
        std::tuple {tr("Files to repair:"), u"repairPreviewChanged"_s, m_changedFiles, detailsSummary},
        std::tuple {tr("Oversized files:"), u"repairPreviewOversized"_s, m_oversizedFiles, detailsSummary}})
    {
        value->setObjectName(name);
        value->setTextInteractionFlags(Qt::TextSelectableByMouse);
        value->setWordWrap(true);
        value->setText(tr("Not analyzed"));
        target->addRow(label, value);
    }
    detailsLayout->addWidget(m_detailsSummary);

    m_files->setObjectName(u"repairPreviewFiles"_s);
    m_files->setColumnCount(6);
    m_files->setHorizontalHeaderLabels({tr("Target file"), tr("Chosen source"), tr("Expected bytes")
        , tr("Candidate bytes"), tr("Verified bytes"), tr("Problems")});
    m_files->setAlternatingRowColors(true);
    m_files->setSelectionBehavior(QAbstractItemView::SelectRows);
    m_files->setSelectionMode(QAbstractItemView::SingleSelection);
    m_files->setEditTriggers(QAbstractItemView::NoEditTriggers);
    m_files->verticalHeader()->hide();
    m_files->horizontalHeader()->setSectionResizeMode(0, QHeaderView::Stretch);
    m_files->horizontalHeader()->setSectionResizeMode(1, QHeaderView::Stretch);
    for (int column = 2; column < 6; ++column)
        m_files->horizontalHeader()->setSectionResizeMode(column, QHeaderView::ResizeToContents);
    m_files->setMinimumHeight(180);
    detailsLayout->addWidget(m_files, 1);
    m_chooseSource = new QPushButton {tr("Choose source for selected file…"), this};
    m_chooseSource->setObjectName(u"repairPreviewChooseSource"_s);
    detailsLayout->addWidget(m_chooseSource, 0, Qt::AlignLeft);
    details->hide();
    layout->addWidget(details);
    connect(detailsToggle, &QToolButton::toggled, this, [this, details, detailsToggle](const bool expanded)
    {
        details->setVisible(expanded);
        detailsToggle->setArrowType(expanded ? Qt::DownArrow : Qt::RightArrow);
        this->layout()->activate();
        resize(width(), sizeHint().height());
    });
    m_reviewed->setObjectName(u"repairPreviewReviewed"_s);
    layout->addWidget(m_reviewed);

    auto *buttons = new QDialogButtonBox {QDialogButtonBox::Close, this};
    m_previewButton = buttons->addButton(tr("Scan files"), QDialogButtonBox::ActionRole);
    m_previewButton->setObjectName(u"repairPreviewAnalyze"_s);
    m_previewButton->setDefault(true);
    m_applyButton = buttons->addButton(tr("Continue"), QDialogButtonBox::ActionRole);
    m_applyButton->setObjectName(u"repairPreviewApply"_s);
    m_applyButton->setAutoDefault(false);
    m_closeButton = buttons->button(QDialogButtonBox::Close);
    layout->addWidget(buttons);

    connect(m_torrentFile, &FileSystemPathEdit::selectedPathChanged, this, &RepairPreviewDialog::loadTarget);
    connect(m_destination, &FileSystemPathEdit::selectedPathChanged, this, &RepairPreviewDialog::clearPreview);
    connect(m_sourceDirectories, &QPlainTextEdit::textChanged, this, &RepairPreviewDialog::clearPreview);
    connect(m_addSourceDirectory, &QPushButton::clicked, this, [this]
    {
        const QString source = QFileDialog::getExistingDirectory(this, tr("Choose a source directory"));
        if (!source.isEmpty())
            m_sourceDirectories->appendPlainText(QDir::fromNativeSeparators(source));
    });
    connect(m_mode, &QComboBox::currentIndexChanged, this, &RepairPreviewDialog::clearPreview);
    connect(m_reviewed, &QCheckBox::toggled, this, &RepairPreviewDialog::updateControls);
    connect(m_files, &QTableWidget::itemSelectionChanged, this, &RepairPreviewDialog::updateControls);
    connect(m_files, &QTableWidget::itemChanged, this, [this](const QTableWidgetItem *item)
    {
        if (item->column() == 0)
            clearPreview();
    });
    connect(m_chooseSource, &QPushButton::clicked, this, &RepairPreviewDialog::chooseSource);
    connect(m_previewButton, &QPushButton::clicked, this, &RepairPreviewDialog::preview);
    connect(m_applyButton, &QPushButton::clicked, this, &RepairPreviewDialog::startRepair);
    connect(buttons, &QDialogButtonBox::rejected, this, &RepairPreviewDialog::reject);
    connect(&m_previewWatcher, &QFutureWatcherBase::finished, this, [this]
    {
        m_operation = Operation::Idle;
        RepairPlan plan = m_previewWatcher.future().takeResult();
        if (m_cancelled->load(std::memory_order_relaxed))
        {
            if (!m_closePending)
                clearPreview();
        }
        else if (plan.error.isEmpty())
        {
            m_plan = std::move(plan);
            showPreview();
        }
        else if (!m_closePending)
        {
            m_plan.reset();
            m_status->setText(tr("Scan failed: %1").arg(plan.error));
        }
        updateControls();
        if (m_closePending)
            QDialog::reject();
    });
    updateControls();
}

RepairPreviewDialog::~RepairPreviewDialog()
{
    if (m_cancelled)
        m_cancelled->store(true, std::memory_order_relaxed);
    m_worker.waitForDone();
}

void RepairPreviewDialog::reject()
{
    if (m_operation == Operation::Preview)
    {
        m_closePending = true;
        m_cancelled->store(true, std::memory_order_relaxed);
        m_status->setText(tr("Cancelling scan…"));
        updateControls();
        return;
    }
    if (m_operation != Operation::Idle)
        return;
    QDialog::reject();
}

QStringList RepairPreviewDialog::sourceRoots() const
{
    QStringList roots;
    for (const QString &source : m_sourceDirectories->toPlainText().split(u'\n', Qt::SkipEmptyParts))
        roots.append(QDir::cleanPath(QDir::fromNativeSeparators(source.trimmed())));
    return roots;
}

void RepairPreviewDialog::clearPreview()
{
    if (m_operation == Operation::Preview)
        m_cancelled->store(true, std::memory_order_relaxed);
    m_plan.reset();
    m_reviewed->setChecked(false);
    const QSignalBlocker blocker {m_files};
    for (int row = 0; row < m_files->rowCount(); ++row)
    {
        for (const int column : {1, 3, 4, 5})
            m_files->item(row, column)->setText(tr("Not analyzed"));
    }
    for (QLabel *value : {m_candidateBytes, m_verifiedBytes, m_networkBytes, m_temporaryBytes, m_changedFiles, m_oversizedFiles})
        value->setText(tr("Not analyzed"));
    m_status->setText(m_descriptor && selectedFiles().isEmpty() ? tr("Select at least one file in Files and options.")
        : tr("Scanning does not change your files."));
    updateControls();
}

void RepairPreviewDialog::loadTarget()
{
    m_descriptor.reset();
    m_explicitMappings.clear();
    const QSignalBlocker blocker {m_files};
    m_files->setRowCount(0);
    clearPreview();
    const Path torrentPath = m_torrentFile->selectedPath();
    if (!torrentPath.isAbsolute() || !QFileInfo(torrentPath.toString()).isFile())
        return;
    const auto descriptor = TorrentDescriptor::loadFromFile(torrentPath);
    if (!descriptor || !descriptor->info())
    {
        m_status->setText(tr("Cannot load target torrent metadata: %1").arg(descriptor ? tr("metadata is missing") : descriptor.error()));
        return;
    }
    m_descriptor = *descriptor;
    const TorrentInfo &info = *descriptor->info();
    const auto indexes = info.nativeIndexes();
    m_files->setRowCount(info.filesCount());
    for (int row = 0; row < info.filesCount(); ++row)
    {
        for (int column = 0; column < m_files->columnCount(); ++column)
            m_files->setItem(row, column, new QTableWidgetItem);
        auto *target = m_files->item(row, 0);
        target->setText(QDir::fromNativeSeparators(info.filePath(row).toString()));
        target->setData(Qt::UserRole, int(indexes.at(row)));
        target->setFlags(target->flags() | Qt::ItemIsUserCheckable);
        target->setCheckState(Qt::Checked);
        m_files->item(row, 2)->setText(locale().toString(info.fileSize(row)));
    }
    clearPreview();
}

QSet<int> RepairPreviewDialog::selectedFiles() const
{
    QSet<int> selected;
    for (int row = 0; row < m_files->rowCount(); ++row)
    {
        const auto *item = m_files->item(row, 0);
        if (item->checkState() == Qt::Checked)
            selected.insert(item->data(Qt::UserRole).toInt());
    }
    return selected;
}

void RepairPreviewDialog::preview()
{
    if ((m_operation != Operation::Idle) || !m_descriptor || selectedFiles().isEmpty())
        return;
    clearPreview();
    const QString destination = QDir::cleanPath(QDir::fromNativeSeparators(m_destination->selectedPath().toString()));
    if (!QDir::isAbsolutePath(destination) || !QFileInfo(destination).isDir())
    {
        m_status->setText(tr("Choose an existing folder for the torrent's files."));
        return;
    }
    const QStringList roots = (m_mode->currentIndex() == 0) ? sourceRoots() : QStringList {};
    for (const QString &root : roots)
    {
        if (!QDir::isAbsolutePath(root) || !QFileInfo(root).isDir())
        {
            m_status->setText(tr("Each source folder must exist and have an absolute path."));
            return;
        }
    }

    const auto target = m_descriptor->info()->nativeInfo();
    const lt::file_storage files = target->files();
    m_operation = Operation::Preview;
    m_cancelled = std::make_shared<std::atomic_bool>(false);
    m_status->setText(tr("Scanning files…"));
    updateControls();
    const QMap<int, QString> mappings = (m_mode->currentIndex() == 0) ? m_explicitMappings : QMap<int, QString> {};
    m_previewWatcher.setFuture(QtConcurrent::run(&m_worker, [target, files, destination, roots
        , mappings, selected = selectedFiles(), cancelled = m_cancelled]
    {
        return planRepairData(*target, files, destination, roots, mappings, selected, cancelled.get());
    }));
}

void RepairPreviewDialog::chooseSource()
{
    if ((m_operation != Operation::Idle) || (m_files->currentRow() < 0))
        return;
    const int nativeIndex = m_files->item(m_files->currentRow(), 0)->data(Qt::UserRole).toInt();
    const QString source = QFileDialog::getOpenFileName(this, tr("Choose a source file"));
    if (source.isEmpty())
        return;
    m_explicitMappings.insert(nativeIndex, QDir::cleanPath(QDir::fromNativeSeparators(source)));
    clearPreview();
    m_status->setText(tr("Source changed. Scan again to update the results."));
}

void RepairPreviewDialog::showPreview()
{
    Q_ASSERT(m_plan);
    const auto formatBytes = [this](const qint64 bytes)
    {
        return tr("%L1 bytes (%2)").arg(bytes).arg(locale().formattedDataSize(bytes));
    };
    const QSignalBlocker blocker {m_files};
    for (int row = 0; row < m_plan->files.size(); ++row)
    {
        const RepairPlanFile &file = m_plan->files.at(row);
        const QStringList values {file.targetPath, file.sourcePath.isEmpty() ? tr("No candidate selected") : file.sourcePath
            , locale().toString(file.expectedBytes), locale().toString(file.candidateBytes)
            , locale().toString(file.verifiedBytes), file.problems.join(u'\n')};
        for (int column = 0; column < values.size(); ++column)
        {
            auto *item = m_files->item(row, column);
            item->setText(values.at(column));
            item->setToolTip(values.at(column));
            if ((column >= 2) && (column <= 4))
                item->setTextAlignment(Qt::AlignRight | Qt::AlignVCenter);
        }
        m_files->item(row, 0)->setData(Qt::UserRole, file.nativeIndex);
    }
    m_candidateBytes->setText(formatBytes(m_plan->candidateBytes));
    m_verifiedBytes->setText(formatBytes(m_plan->verifiedBytes));
    m_networkBytes->setText(formatBytes(m_plan->requiredNetworkBytes));
    m_temporaryBytes->setText(tr("%1 required; %2 available")
        .arg(formatBytes(m_plan->temporaryStorageBytes), formatBytes(m_plan->availableStorageBytes)));
    m_changedFiles->setText(locale().toString(m_plan->changedFiles));
    m_oversizedFiles->setText(locale().toString(m_plan->oversizedFiles));
    m_status->setText((m_mode->currentIndex() == 0)
        ? tr("Repair a separate copy first. Originals stay unchanged until you confirm replacement.")
        : tr("Original files will be changed without a backup. You will confirm this before repair starts."));
    m_reviewed->setChecked(false);
}

void RepairPreviewDialog::startRepair()
{
    if ((m_operation != Operation::Idle) || !m_plan || !m_descriptor || !m_reviewed->isChecked()
        || selectedFiles().isEmpty())
        return;
    if ((m_mode->currentIndex() == 0) && (m_plan->temporaryStorageBytes > m_plan->availableStorageBytes))
    {
        m_status->setText(tr("Not enough space for a separate copy: %1 needed, %2 available. Choose another folder.")
            .arg(locale().formattedDataSize(m_plan->temporaryStorageBytes), locale().formattedDataSize(m_plan->availableStorageBytes)));
        return;
    }
    Session *session = Session::instance();
    if (session->findTorrent(m_descriptor->infoHash()))
    {
        m_status->setText(tr("This torrent already exists. Stop it and use Smart repair from its transfer-list menu."));
        return;
    }

    AddTorrentParams params;
    params.savePath = m_destination->selectedPath();
    params.useAutoTMM = false;
    params.useDownloadPath = false;
    params.addStopped = true;
    params.contentLayout = TorrentContentLayout::Original;
    params.filePaths = m_descriptor->info()->filePaths();
    const QSet<int> selected = selectedFiles();
    for (const lt::file_index_t index : m_descriptor->info()->nativeIndexes())
        params.filePriorities.append(selected.contains(int(index)) ? DownloadPriority::Normal : DownloadPriority::Ignored);
    const InfoHash expectedHash = m_descriptor->infoHash();
    const Path expectedSavePath = params.savePath;
    const PathList expectedFilePaths = params.filePaths;
    const QList<DownloadPriority> expectedPriorities = params.filePriorities;
    const bool staged = (m_mode->currentIndex() == 0);
    const QStringList roots = staged ? sourceRoots() : QStringList {};
    const QMap<int, QString> mappings = staged ? m_plan->mappings : QMap<int, QString> {};
    const RepairDialogMode mode = staged ? RepairDialogMode::Staged : RepairDialogMode::InPlace;
    disconnect(m_torrentAddedConnection);
    disconnect(m_addTorrentFailedConnection);
    m_torrentAddedConnection = connect(session, &Session::torrentAdded, this
        , [this, expectedHash, expectedSavePath, expectedFilePaths, expectedPriorities, roots, mappings, mode](Torrent *torrent)
    {
        if ((torrent->infoHash() != expectedHash) || !torrent->isStopped() || torrent->isAutoTMMEnabled()
            || !torrent->downloadPath().isEmpty() || (torrent->savePath() != expectedSavePath)
            || (torrent->filePaths() != expectedFilePaths) || (torrent->filePriorities() != expectedPriorities))
            return;
        disconnect(m_torrentAddedConnection);
        disconnect(m_addTorrentFailedConnection);
        m_operation = Operation::Idle;
        auto *dialog = new RepairDialog {parentWidget(), torrent, {mode, roots, mappings, true}};
        dialog->setAttribute(Qt::WA_DeleteOnClose);
        dialog->open();
        accept();
    });
    m_addTorrentFailedConnection = connect(session, &Session::addTorrentFailed, this
        , [this, expectedHash](const InfoHash &hash, const AddTorrentError &error)
    {
        if (hash != expectedHash)
            return;
        disconnect(m_torrentAddedConnection);
        disconnect(m_addTorrentFailedConnection);
        m_operation = Operation::Idle;
        m_status->setText(tr("The stopped repair job was not added: %1").arg(error.message));
        updateControls();
    });
    m_operation = Operation::Adding;
    m_status->setText(tr("Adding the torrent…"));
    updateControls();
    if (!session->addTorrent(*m_descriptor, params))
    {
        disconnect(m_torrentAddedConnection);
        disconnect(m_addTorrentFailedConnection);
        m_operation = Operation::Idle;
        m_status->setText(tr("The stopped repair job was rejected before it was added."));
        updateControls();
    }
}

void RepairPreviewDialog::updateControls()
{
    const bool idle = (m_operation == Operation::Idle);
    const bool planned = idle && m_plan.has_value();
    const bool staged = (m_mode->currentIndex() == 0);
    m_torrentFile->setEnabled(idle);
    m_destination->setEnabled(idle);
    m_sourceDirectories->setEnabled(idle && staged);
    m_addSourceDirectory->setEnabled(idle && staged);
    m_mode->setEnabled(idle);
    m_files->setEnabled(idle);
    m_progress->setVisible(!idle);
    m_results->setVisible(m_plan.has_value());
    m_detailsSummary->setVisible(m_plan.has_value());
    m_reviewed->setVisible(m_plan.has_value());
    m_applyButton->setVisible(m_plan.has_value());
    m_previewButton->setText(m_plan ? tr("Scan again") : tr("Scan files"));
    m_previewButton->setEnabled(idle && m_descriptor.has_value() && !selectedFiles().isEmpty()
        && m_destination->selectedPath().isAbsolute());
    m_chooseSource->setEnabled(idle && staged && (m_files->currentRow() >= 0));
    m_reviewed->setEnabled(planned);
    m_applyButton->setEnabled(planned && m_reviewed->isChecked());
    m_closeButton->setEnabled(idle || (m_operation == Operation::Preview));
}
