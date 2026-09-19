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
#include <QTableWidget>
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
    , m_reviewed {new QCheckBox {tr("I reviewed the mappings and allow adding a stopped job and creating missing empty target files."), this}}
{
    setObjectName(u"RepairPreviewDialog"_s);
    setWindowTitle(tr("Smart repair from torrent file"));
    setWindowModality(Qt::WindowModal);
    resize(1040, 720);
    m_worker.setMaxThreadCount(1);

    auto *layout = new QVBoxLayout {this};
    auto *introduction = new QLabel {tr("Select a .torrent and the parent directory where its paths belong. "
        "Preview reads only the chosen locations and includes every target file. "
        "It does not add a torrent, create resume data, truncate files or start a download."), this};
    introduction->setWordWrap(true);
    layout->addWidget(introduction);

    m_torrentFile->setObjectName(u"repairPreviewTorrent"_s);
    m_torrentFile->setMode(FileSystemPathEdit::Mode::FileOpen);
    m_torrentFile->setDialogCaption(tr("Choose target torrent"));
    m_torrentFile->setFileNameFilter(tr("Torrent files (*.torrent)"));
    m_destination->setObjectName(u"repairPreviewDestination"_s);
    m_destination->setMode(FileSystemPathEdit::Mode::DirectoryOpen);
    m_destination->setDialogCaption(tr("Choose target parent directory"));
    m_sourceDirectories->setObjectName(u"repairPreviewRoots"_s);
    m_sourceDirectories->setPlaceholderText(tr("Optional source directories, one per line. No other locations are searched."));
    m_sourceDirectories->setMaximumHeight(70);
    m_mode->setObjectName(u"repairPreviewMode"_s);
    m_mode->addItems({tr("Safe staged update"), tr("Repair in place, without rollback")});
    auto *sources = new QFormLayout;
    sources->setRowWrapPolicy(QFormLayout::DontWrapRows);
    sources->addRow(tr("Target .torrent:"), m_torrentFile);
    sources->addRow(tr("Target parent directory:"), m_destination);
    sources->addRow(tr("Find existing data:"), m_sourceDirectories);
    m_addSourceDirectory = new QPushButton {tr("Add source directory…"), this};
    m_addSourceDirectory->setObjectName(u"repairPreviewAddRoot"_s);
    sources->addRow(QString {}, m_addSourceDirectory);
    sources->addRow(tr("Operation after preview:"), m_mode);
    layout->addLayout(sources);

    m_status->setObjectName(u"repairPreviewStatus"_s);
    m_status->setTextFormat(Qt::PlainText);
    m_status->setTextInteractionFlags(Qt::TextSelectableByMouse);
    m_status->setWordWrap(true);
    m_status->setText(tr("Choose the target torrent and directory, then run a read-only preview."));
    layout->addWidget(m_status);
    m_progress->setObjectName(u"repairPreviewProgress"_s);
    m_progress->setRange(0, 0);
    m_progress->setTextVisible(false);
    layout->addWidget(m_progress);

    for (auto [label, name, value] : {
        std::tuple {tr("Candidate bytes found:"), u"repairPreviewCandidates"_s, m_candidateBytes},
        std::tuple {tr("Verified bytes reusable:"), u"repairPreviewVerified"_s, m_verifiedBytes},
        std::tuple {tr("Target payload required from network:"), u"repairPreviewNetwork"_s, m_networkBytes},
        std::tuple {tr("Temporary space for safe staging:"), u"repairPreviewTemporary"_s, m_temporaryBytes},
        std::tuple {tr("Target files needing data or size repair:"), u"repairPreviewChanged"_s, m_changedFiles},
        std::tuple {tr("Files with extra tails:"), u"repairPreviewOversized"_s, m_oversizedFiles}})
    {
        value->setObjectName(name);
        value->setTextInteractionFlags(Qt::TextSelectableByMouse);
        value->setWordWrap(true);
        value->setText(tr("Not analyzed"));
        sources->addRow(label, value);
    }

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
    layout->addWidget(m_files, 1);

    auto *mappingNotice = new QLabel {tr("Every chosen source is shown above before any write. Candidate bytes are only data found for checking; "
        "only hash-verified bytes count as reusable. Network bytes are target payload and exclude protocol or duplicate traffic. "
        "Files outside the torrent manifest are preserved."), this};
    mappingNotice->setWordWrap(true);
    layout->addWidget(mappingNotice);
    m_reviewed->setObjectName(u"repairPreviewReviewed"_s);
    layout->addWidget(m_reviewed);

    auto *buttons = new QDialogButtonBox {QDialogButtonBox::Close, this};
    m_chooseSource = buttons->addButton(tr("Choose source for selected target…"), QDialogButtonBox::ActionRole);
    m_chooseSource->setObjectName(u"repairPreviewChooseSource"_s);
    m_previewButton = buttons->addButton(tr("Preview without changes"), QDialogButtonBox::ActionRole);
    m_previewButton->setObjectName(u"repairPreviewAnalyze"_s);
    m_applyButton = buttons->addButton(tr("Add stopped and continue"), QDialogButtonBox::ActionRole);
    m_applyButton->setObjectName(u"repairPreviewApply"_s);
    m_applyButton->setAutoDefault(false);
    m_closeButton = buttons->button(QDialogButtonBox::Close);
    layout->addWidget(buttons);

    connect(m_torrentFile, &FileSystemPathEdit::selectedPathChanged, this, [this]
    {
        m_explicitMappings.clear();
        clearPreview();
    });
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
    connect(m_chooseSource, &QPushButton::clicked, this, &RepairPreviewDialog::chooseSource);
    connect(m_previewButton, &QPushButton::clicked, this, &RepairPreviewDialog::preview);
    connect(m_applyButton, &QPushButton::clicked, this, &RepairPreviewDialog::startRepair);
    connect(buttons, &QDialogButtonBox::rejected, this, &RepairPreviewDialog::reject);
    connect(&m_previewWatcher, &QFutureWatcherBase::finished, this, [this]
    {
        m_operation = Operation::Idle;
        RepairPlan plan = m_previewWatcher.future().takeResult();
        if (plan.error.isEmpty())
        {
            m_plan = std::move(plan);
            showPreview();
        }
        else if (!m_closePending)
        {
            m_plan.reset();
            m_status->setText(tr("Preview refused: %1").arg(plan.error));
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
        m_status->setText(tr("Cancelling the read-only preview…"));
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
    if (m_operation != Operation::Idle)
        return;
    m_descriptor.reset();
    m_plan.reset();
    m_reviewed->setChecked(false);
    m_files->setRowCount(0);
    for (QLabel *value : {m_candidateBytes, m_verifiedBytes, m_networkBytes, m_temporaryBytes, m_changedFiles, m_oversizedFiles})
        value->setText(tr("Not analyzed"));
    m_status->setText(tr("Choose the target torrent and directory, then run a read-only preview."));
    updateControls();
}

void RepairPreviewDialog::preview()
{
    if (m_operation != Operation::Idle)
        return;
    clearPreview();
    const Path torrentPath = m_torrentFile->selectedPath();
    const QString destination = QDir::cleanPath(QDir::fromNativeSeparators(m_destination->selectedPath().toString()));
    if (!torrentPath.isAbsolute() || !QFileInfo(torrentPath.toString()).isFile()
        || !QDir::isAbsolutePath(destination) || !QFileInfo(destination).isDir())
    {
        m_status->setText(tr("Choose an existing .torrent file and an ordinary target parent directory."));
        return;
    }
    const auto descriptor = TorrentDescriptor::loadFromFile(torrentPath);
    if (!descriptor || !descriptor->info())
    {
        m_status->setText(tr("Cannot load target torrent metadata: %1").arg(descriptor ? tr("metadata is missing") : descriptor.error()));
        return;
    }
    const QStringList roots = (m_mode->currentIndex() == 0) ? sourceRoots() : QStringList {};
    for (const QString &root : roots)
    {
        if (!QDir::isAbsolutePath(root) || !QFileInfo(root).isDir())
        {
            m_status->setText(tr("Every source directory must be an existing absolute directory."));
            return;
        }
    }

    m_descriptor = *descriptor;
    const auto target = descriptor->info()->nativeInfo();
    const lt::file_storage files = target->files();
    m_operation = Operation::Preview;
    m_cancelled = std::make_shared<std::atomic_bool>(false);
    m_status->setText(tr("Checking selected paths and torrent hashes. No files are changed."));
    updateControls();
    const QMap<int, QString> mappings = (m_mode->currentIndex() == 0) ? m_explicitMappings : QMap<int, QString> {};
    m_previewWatcher.setFuture(QtConcurrent::run(&m_worker, [target, files, destination, roots
        , mappings, cancelled = m_cancelled]
    {
        return planRepairData(*target, files, destination, roots, mappings, cancelled.get());
    }));
}

void RepairPreviewDialog::chooseSource()
{
    if ((m_operation != Operation::Idle) || (m_files->currentRow() < 0))
        return;
    const int nativeIndex = m_files->item(m_files->currentRow(), 0)->data(Qt::UserRole).toInt();
    const QString source = QFileDialog::getOpenFileName(this, tr("Choose existing bytes for this target file"));
    if (source.isEmpty())
        return;
    m_explicitMappings.insert(nativeIndex, QDir::cleanPath(QDir::fromNativeSeparators(source)));
    clearPreview();
    m_status->setText(tr("The source mapping changed. Run the read-only preview again."));
}

void RepairPreviewDialog::showPreview()
{
    Q_ASSERT(m_plan);
    const auto formatBytes = [this](const qint64 bytes)
    {
        return tr("%L1 bytes (%2)").arg(bytes).arg(locale().formattedDataSize(bytes));
    };
    m_files->setRowCount(m_plan->files.size());
    for (int row = 0; row < m_plan->files.size(); ++row)
    {
        const RepairPlanFile &file = m_plan->files.at(row);
        const QStringList values {file.targetPath, file.sourcePath.isEmpty() ? tr("No candidate selected") : file.sourcePath
            , locale().toString(file.expectedBytes), locale().toString(file.candidateBytes)
            , locale().toString(file.verifiedBytes), file.problems.join(u'\n')};
        for (int column = 0; column < values.size(); ++column)
        {
            auto *item = new QTableWidgetItem {values.at(column)};
            item->setToolTip(values.at(column));
            if ((column >= 2) && (column <= 4))
                item->setTextAlignment(Qt::AlignRight | Qt::AlignVCenter);
            m_files->setItem(row, column, item);
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
    const QString firstTarget = m_plan->files.isEmpty() ? QString {} : QDir(m_destination->selectedPath().toString())
        .filePath(m_plan->files.constFirst().targetPath);
    m_status->setText(tr("Read-only preview complete. First resolved target: %1\n"
        "Mappings will be checked again under exclusive repair ownership before any payload write.").arg(firstTarget));
    m_reviewed->setChecked(false);
}

void RepairPreviewDialog::startRepair()
{
    if ((m_operation != Operation::Idle) || !m_plan || !m_descriptor || !m_reviewed->isChecked())
        return;
    if ((m_mode->currentIndex() == 0) && (m_plan->temporaryStorageBytes > m_plan->availableStorageBytes))
    {
        m_status->setText(tr("Safe staging does not have enough temporary space. Choose another destination or explicitly select in-place repair."));
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
    params.filePriorities.fill(DownloadPriority::Normal, params.filePaths.size());
    const InfoHash expectedHash = m_descriptor->infoHash();
    const Path expectedSavePath = params.savePath;
    const PathList expectedFilePaths = params.filePaths;
    const bool staged = (m_mode->currentIndex() == 0);
    const QStringList roots = staged ? sourceRoots() : QStringList {};
    const QMap<int, QString> mappings = staged ? m_plan->mappings : QMap<int, QString> {};
    const RepairDialogMode mode = staged ? RepairDialogMode::Staged : RepairDialogMode::InPlace;
    disconnect(m_torrentAddedConnection);
    disconnect(m_addTorrentFailedConnection);
    m_torrentAddedConnection = connect(session, &Session::torrentAdded, this
        , [this, expectedHash, expectedSavePath, expectedFilePaths, roots, mappings, mode](Torrent *torrent)
    {
        if ((torrent->infoHash() != expectedHash) || !torrent->isStopped() || torrent->isAutoTMMEnabled()
            || !torrent->downloadPath().isEmpty() || (torrent->savePath() != expectedSavePath)
            || (torrent->filePaths() != expectedFilePaths))
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
    m_status->setText(tr("Adding a stopped, manually managed repair job. Initialization may create missing empty target files; existing files are preserved. No payload download is started."));
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
    m_previewButton->setEnabled(idle && m_torrentFile->selectedPath().isAbsolute() && m_destination->selectedPath().isAbsolute());
    m_chooseSource->setEnabled(planned && staged && (m_files->currentRow() >= 0));
    m_reviewed->setEnabled(planned);
    m_applyButton->setEnabled(planned && m_reviewed->isChecked());
    m_closeButton->setEnabled(idle || (m_operation == Operation::Preview));
}
