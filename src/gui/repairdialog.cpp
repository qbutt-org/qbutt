/*
 * qbutt, a native Qt BitTorrent client.
 * Copyright (C) 2026 qbutt contributors
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the
 * Free Software Foundation; either version 2 of the License, or (at your
 * option) any later version.
 */

#include "repairdialog.h"

#include <QCheckBox>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QFormLayout>
#include <QHeaderView>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QLocale>
#include <QPlainTextEdit>
#include <QProgressBar>
#include <QPushButton>
#include <QTextDocument>
#include <QTimer>
#include <QToolButton>
#include <QTreeWidget>
#include <QVBoxLayout>

#include "base/bittorrent/infohash.h"
#include "base/bittorrent/repairanalysis.h"
#include "base/bittorrent/repairservice.h"
#include "base/bittorrent/session.h"
#include "base/bittorrent/stagingoperation.h"
#include "base/bittorrent/torrent.h"
#include "base/bittorrent/torrentinfo.h"
#include "base/path.h"

RepairDialog::RepairDialog(QWidget *parent, BitTorrent::Torrent *torrent, const RepairDialogOptions &options)
    : QDialog {parent}
    , m_service {new BitTorrent::RepairService {torrent, this}}
    , m_status {new QLabel {this}}
    , m_progress {new QProgressBar {this}}
    , m_files {new QTreeWidget {this}}
    , m_consent {new QCheckBox {tr("I have closed other programs that can change these files."), this}}
{
    setObjectName(QStringLiteral("RepairDialog"));
    setWindowTitle(tr("Repair files"));
    setWindowModality(Qt::WindowModal);
    resize(760, 0);

    auto *layout = new QVBoxLayout {this};
    auto *details = new QFormLayout;
    auto *name = new QLabel {torrent->name(), this};
    name->setTextFormat(Qt::PlainText);
    name->setTextInteractionFlags(Qt::TextSelectableByMouse);
    name->setWordWrap(true);
    details->addRow(tr("Torrent:"), name);
    const QString pendingDestination = BitTorrent::StagingOperation::pendingDestination(torrent->id().toString());
    auto *location = new QLabel {pendingDestination.isEmpty() ? torrent->actualStorageLocation().toString() : pendingDestination, this};
    location->setTextFormat(Qt::PlainText);
    location->setTextInteractionFlags(Qt::TextSelectableByMouse);
    location->setWordWrap(true);
    details->addRow(tr("Folder:"), location);
    layout->addLayout(details);

    auto *detailsToggle = new QToolButton {this};
    detailsToggle->setObjectName(QStringLiteral("repairDetails"));
    detailsToggle->setText(tr("Files and options"));
    detailsToggle->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
    detailsToggle->setArrowType(Qt::RightArrow);
    detailsToggle->setCheckable(true);
    auto *optionsWidget = new QWidget {this};
    auto *optionsLayout = new QVBoxLayout {optionsWidget};
    optionsLayout->setContentsMargins(0, 0, 0, 0);
    auto *settings = new QFormLayout;
    auto *mode = new QComboBox {this};
    mode->setObjectName(QStringLiteral("repairMode"));
    mode->addItems({tr("Repair a separate copy first"), tr("Change original files, without a backup"), tr("Resume interrupted repair")});
    if (options.mode == RepairDialogMode::RecoverStaged
        || QFileInfo::exists(BitTorrent::StagingOperation::journalPath(torrent->id().toString())))
        mode->setCurrentIndex(2);
    else if (options.mode == RepairDialogMode::InPlace)
        mode->setCurrentIndex(1);
    settings->addRow(tr("Repair method:"), mode);
    auto *roots = new QPlainTextEdit {this};
    roots->setObjectName(QStringLiteral("repairSourceRoots"));
    roots->setPlaceholderText(tr("Other folders to search, one per line"));
    roots->setMaximumHeight(64);
    roots->setPlainText(options.sourceRoots.join(u'\n'));
    m_sourceMappings = options.sourceMappings;
    settings->addRow(tr("Other folders:"), roots);
    auto *browse = new QPushButton {tr("Add folder…"), this};
    settings->addRow(QString {}, browse);
    optionsLayout->addLayout(settings);
    auto *chooseSource = new QPushButton {tr("Choose source for selected file…"), this};
    connect(chooseSource, &QPushButton::clicked, this, [this]
    {
        QTreeWidgetItem *item = m_files->currentItem();
        if (!item)
            return;
        const QString source = QFileDialog::getOpenFileName(this, tr("Choose a source file"));
        if (source.isEmpty())
            return;
        m_sourceMappings.insert(item->data(0, Qt::UserRole).toInt(), QDir::fromNativeSeparators(source));
        item->setText(4, tr("Source: %1").arg(source));
    });
    connect(browse, &QPushButton::clicked, this, [this, roots]
    {
        const QString source = QFileDialog::getExistingDirectory(this, tr("Choose a source directory"));
        if (!source.isEmpty())
            roots->appendPlainText(QDir::fromNativeSeparators(source));
    });
    m_status->setTextFormat(Qt::PlainText);
    m_status->setObjectName(QStringLiteral("repairStatus"));
    m_status->setTextInteractionFlags(Qt::TextSelectableByMouse);
    m_status->setWordWrap(true);
    m_status->setText((mode->currentIndex() == 2) ? tr("An interrupted repair is available to resume.")
        : tr("Scanning does not change your files."));
    layout->addWidget(m_status);

    m_progress->setRange(0, 0);
    m_progress->setObjectName(QStringLiteral("repairProgress"));
    m_progress->setTextVisible(false);
    m_progress->hide();
    layout->addWidget(m_progress);

    m_files->setHeaderLabels({tr("File"), tr("Expected bytes"), tr("Actual bytes"), tr("Verified bytes"), tr("Problems")});
    m_files->setObjectName(QStringLiteral("repairFiles"));
    m_files->setRootIsDecorated(false);
    m_files->setAlternatingRowColors(true);
    m_files->setSelectionMode(QAbstractItemView::ExtendedSelection);
    m_files->setEditTriggers(QAbstractItemView::NoEditTriggers);
    m_files->header()->setSectionResizeMode(0, QHeaderView::Stretch);
    for (int column = 1; column <= 3; ++column)
        m_files->header()->setSectionResizeMode(column, QHeaderView::ResizeToContents);
    m_files->setMinimumHeight(180);
    optionsLayout->addWidget(m_files, 1);
    optionsLayout->addWidget(chooseSource, 0, Qt::AlignLeft);
    for (int index = 0; index < torrent->filesCount(); ++index)
    {
        auto *item = new QTreeWidgetItem {m_files, {torrent->filePath(index).toString()
            , locale().toString(torrent->fileSize(index)), tr("Not analyzed"), QString {}, QString {}}};
        item->setData(0, Qt::UserRole, int(torrent->info().nativeIndexes().at(index)));
    }

    layout->addWidget(detailsToggle, 0, Qt::AlignLeft);
    optionsWidget->hide();
    layout->addWidget(optionsWidget);
    connect(detailsToggle, &QToolButton::toggled, this, [this, optionsWidget, detailsToggle](const bool expanded)
    {
        optionsWidget->setVisible(expanded);
        detailsToggle->setArrowType(expanded ? Qt::DownArrow : Qt::RightArrow);
        this->layout()->activate();
        resize(width(), sizeHint().height());
    });
    m_consent->setEnabled(false);
    m_consent->setObjectName(QStringLiteral("repairConsent"));
    m_consent->hide();
    layout->addWidget(m_consent);

    auto *buttons = new QDialogButtonBox {QDialogButtonBox::Close, this};
    auto *analyze = buttons->addButton(tr("Scan files"), QDialogButtonBox::ActionRole);
    analyze->setObjectName(QStringLiteral("repairAnalyze"));
    m_apply = buttons->addButton(tr("Repair separate copy"), QDialogButtonBox::ActionRole);
    m_apply->setObjectName(QStringLiteral("repairPrepare"));
    m_commit = buttons->addButton(tr("Replace originals"), QDialogButtonBox::ActionRole);
    m_commit->setObjectName(QStringLiteral("repairCommit"));
    m_rollback = buttons->addButton(tr("Restore originals"), QDialogButtonBox::ActionRole);
    m_rollback->setObjectName(QStringLiteral("repairRollback"));
    m_commit->setEnabled(false);
    m_rollback->setEnabled(false);
    m_apply->setEnabled(false);
    m_apply->setAutoDefault(false);
    m_apply->hide();
    m_commit->hide();
    m_rollback->hide();
    analyze->setDefault(true);
    connect(mode, &QComboBox::currentIndexChanged, this, [analyze, mode]
    {
        analyze->setText((mode->currentIndex() == 2) ? tr("Resume repair") : tr("Scan files"));
    });
    if (mode->currentIndex() == 2)
        analyze->setText(tr("Resume repair"));
    layout->addWidget(buttons);

    const auto updateSources = [this, mode, roots, browse, chooseSource]
    {
        const bool enabled = mode->isEnabled() && (mode->currentIndex() == 0);
        roots->setEnabled(enabled);
        browse->setEnabled(enabled);
        chooseSource->setEnabled(enabled && m_files->currentItem());
    };
    connect(mode, &QComboBox::currentIndexChanged, this, updateSources);
    connect(m_files, &QTreeWidget::currentItemChanged, this, updateSources);
    updateSources();

    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
    connect(m_consent, &QCheckBox::toggled, this, [this](const bool consent)
    {
        const QJsonObject status = m_service->stagingStatus();
        m_apply->setEnabled(consent && (!m_staged || status.value(QStringLiteral("can_prepare")).toBool()));
        m_commit->setEnabled(consent && status.value(QStringLiteral("can_commit")).toBool());
        m_rollback->setEnabled(consent && status.value(QStringLiteral("can_rollback")).toBool());
    });
    connect(analyze, &QPushButton::clicked, this, [this, mode, roots, analyze, updateSources]
    {
        mode->setEnabled(false);
        updateSources();
        analyze->setEnabled(false);
        analyze->hide();
        m_progress->show();
        m_staged = mode->currentIndex() != 1;
        m_status->setText(tr("Scanning files…"));
        m_apply->setText(m_staged ? tr("Repair separate copy") : tr("Repair original files"));
        if (!m_staged)
        {
            m_consent->setText(tr("Other writers are closed. I allow changes without a backup."));
            m_service->analyze();
        }
        else if (mode->currentIndex() == 2)
        {
            m_service->recoverStaged();
        }
        else
        {
            m_service->analyzeStaged(roots->toPlainText().split(u'\n', Qt::SkipEmptyParts), m_sourceMappings);
        }
    });
    connect(m_apply, &QPushButton::clicked, this, [this]()
    {
        m_apply->setEnabled(false);
        m_consent->setEnabled(false);
        m_progress->show();
        m_apply->hide();
        m_consent->hide();
        m_status->setText(m_staged ? tr("Repairing a separate copy. Originals stay unchanged until you confirm replacement.")
            : tr("Repairing original files…"));
        if (m_staged)
            m_service->prepareStaged();
        else
            m_service->apply();
    });
    connect(m_service, &BitTorrent::RepairService::analyzed, this
        , [this, directory = torrent->actualStorageLocation().toString()](const BitTorrent::RepairAnalysis &analysis)
    {
        showAnalysis(analysis, directory);
    });
    connect(m_service, &BitTorrent::RepairService::failed, this, &RepairDialog::showFailure);
    connect(m_service, &BitTorrent::RepairService::recheckStarted, this, [this]()
    {
        m_status->setText(m_staged ? tr("Checking the separate copy before downloading. Close to pause; you can resume later.")
            : tr("Checking repaired files. Closing this window cancels the check."));
    });
    connect(m_service, &BitTorrent::RepairService::recheckFinished, this, [this]()
    {
        m_progress->hide();
        m_status->setText(tr("Check complete. Use Start in the torrent list to download any missing data."));
    });

    connect(m_service, &BitTorrent::RepairService::stagingChanged, this, [this](const QJsonObject &status)
    {
        const QString state = status.value(QStringLiteral("state")).toString();
        const bool canPrepare = status.value(QStringLiteral("can_prepare")).toBool();
        const bool canCommit = status.value(QStringLiteral("can_commit")).toBool();
        const bool canRollback = status.value(QStringLiteral("can_rollback")).toBool();
        m_progress->setVisible(((state == u"downloading") && !canPrepare)
            || (!status.value(QStringLiteral("finalized")).toBool() && !canPrepare && !canCommit && !canRollback));
        m_consent->setChecked(false);
        m_consent->setEnabled(canPrepare || canCommit || canRollback);
        m_consent->setVisible(m_consent->isEnabled());
        m_apply->setVisible(canPrepare);
        m_commit->setVisible(canCommit);
        m_rollback->setVisible(canRollback);
        m_apply->setEnabled(false);
        m_commit->setEnabled(false);
        m_rollback->setEnabled(false);
        m_consent->setText(canCommit ? tr("Other writers are closed. I allow replacing the originals and keeping a backup.")
            : tr("I have closed other programs that can change these files."));
        m_status->setToolTip(tr("Recovery files: %1").arg(status.value(QStringLiteral("payload_path")).toString()));
        if ((state == u"planned") && canPrepare)
        {
            m_status->setText(m_status->text() + tr("\nThe separate copy needs %1 of extra space; %2 is available. Originals stay unchanged until replacement.")
                .arg(locale().formattedDataSize(status.value(QStringLiteral("required_bytes")).toString().toLongLong())
                    , locale().formattedDataSize(status.value(QStringLiteral("available_bytes")).toString().toLongLong())));
        }
        else if (canCommit)
        {
            m_status->setText(tr("The repaired copy is verified and ready to replace the originals. A backup will be kept."));
        }
        else if ((state == u"committed") && status.value(QStringLiteral("finalized")).toBool())
        {
            m_status->setText(tr("Repair complete. The original files are kept in the backup folder."));
        }
        else if ((state == u"rolled_back") && status.value(QStringLiteral("finalized")).toBool())
        {
            m_status->setText(tr("Original files restored. The separate copy is kept."));
        }
        else if (state == u"downloading")
        {
            m_status->setText(canPrepare ? tr("The separate copy is ready to resume repair.")
                : tr("Downloading and checking the separate copy. Close to pause; you can resume later."));
        }
        else if (canRollback)
        {
            m_status->setText(tr("Repair was interrupted. Restore the original files to continue."));
        }
        else
        {
            m_status->setText(tr("Finishing file changes…"));
        }
    });
    connect(m_commit, &QPushButton::clicked, this, [this]
    {
        m_commit->setEnabled(false);
        m_rollback->setEnabled(false);
        m_consent->setEnabled(false);
        m_consent->hide();
        m_commit->hide();
        m_rollback->hide();
        m_progress->show();
        m_status->setText(tr("Replacing original files…"));
        m_service->commitStaged();
    });
    connect(m_rollback, &QPushButton::clicked, this, [this]
    {
        m_commit->setEnabled(false);
        m_rollback->setEnabled(false);
        m_consent->setEnabled(false);
        m_consent->hide();
        m_commit->hide();
        m_rollback->hide();
        m_progress->show();
        m_status->setText(tr("Restoring original files…"));
        m_service->rollbackStaged();
    });
    if (torrent->state() == BitTorrent::TorrentState::CheckingResumeData)
    {
        analyze->setEnabled(false);
        m_progress->show();
        m_status->setText(tr("Preparing to scan…"));
        m_initializationConnection = connect(torrent->session(), &BitTorrent::Session::torrentsUpdated, this
            , [this, hash = torrent->infoHash(), analyze, immediately = options.analyzeImmediately]
        {
            const BitTorrent::Torrent *current = BitTorrent::Session::instance()->findTorrent(hash);
            if (current && (current->state() == BitTorrent::TorrentState::CheckingResumeData))
                return;
            disconnect(m_initializationConnection);
            m_progress->hide();
            if (!current)
            {
                showFailure(tr("The torrent was removed."));
                return;
            }
            analyze->setEnabled(true);
            m_status->setText(tr("Scanning does not change your files."));
            if (immediately)
                analyze->click();
        });
    }
    else if (options.analyzeImmediately)
        QTimer::singleShot(0, analyze, &QPushButton::click);
}

void RepairDialog::showAnalysis(const BitTorrent::RepairAnalysis &analysis, const QString &directory)
{
    m_progress->hide();
    m_files->clear();
    QMap<int, QString> targetNames;
    if (m_staged)
    {
        for (const QJsonValue &value : m_service->stagingStatus().value(QStringLiteral("files")).toArray())
        {
            const QJsonObject file = value.toObject();
            targetNames.insert(file.value(QStringLiteral("index")).toInt(), file.value(QStringLiteral("path")).toString());
        }
    }
    for (const BitTorrent::RepairFileAnalysis &file : analysis.files)
    {
        const QString name = targetNames.value(file.nativeIndex, QDir(directory).relativeFilePath(file.path));
        auto *item = new QTreeWidgetItem {m_files, {name, locale().toString(file.expectedSize)
            , (file.actualSize < 0) ? tr("Missing") : locale().toString(file.actualSize)
            , locale().toString(file.verifiedBytes), file.problems.join(u'\n')}};
        item->setToolTip(0, Qt::convertFromPlainText(m_staged ? tr("Source: %1").arg(file.path) : file.path));
        item->setToolTip(4, Qt::convertFromPlainText(file.problems.join(u'\n')));
        if (!file.selected)
        {
            item->setText(4, tr("Not selected for repair."));
            item->setToolTip(4, tr("Managed repair does not create or truncate this file. Normal downloading may use shared pieces at file boundaries."));
        }
        for (int column = 1; column <= 3; ++column)
            item->setTextAlignment(column, Qt::AlignRight | Qt::AlignVCenter);
    }

    QString summary = tr("Reusable: %1 of %2.")
        .arg(locale().formattedDataSize(analysis.verifiedBytes), locale().formattedDataSize(analysis.expectedBytes));
    if (analysis.wholeFileV2Verification)
        summary += tr("\nPartial files need another check before downloading.");
    if (!m_staged)
        summary += tr("\nRepair will change the original files without a backup.");
    m_status->setText(summary);
    m_consent->setEnabled(true);
    m_consent->show();
    m_apply->show();
}

void RepairDialog::showFailure(const QString &message)
{
    m_progress->hide();
    m_consent->setChecked(false);
    m_consent->setEnabled(false);
    m_apply->setEnabled(false);
    m_apply->hide();
    m_commit->hide();
    m_rollback->hide();
    m_consent->hide();
    m_status->setText(tr("Repair cannot continue: %1").arg(message));
}
