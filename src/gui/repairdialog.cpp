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
#include <QLabel>
#include <QLocale>
#include <QPlainTextEdit>
#include <QProgressBar>
#include <QPushButton>
#include <QTextDocument>
#include <QTreeWidget>
#include <QVBoxLayout>

#include "base/bittorrent/infohash.h"
#include "base/bittorrent/repairanalysis.h"
#include "base/bittorrent/repairservice.h"
#include "base/bittorrent/stagingoperation.h"
#include "base/bittorrent/torrent.h"
#include "base/bittorrent/torrentinfo.h"
#include "base/path.h"

RepairDialog::RepairDialog(QWidget *parent, BitTorrent::Torrent *torrent)
    : QDialog {parent}
    , m_service {new BitTorrent::RepairService {torrent, this}}
    , m_status {new QLabel {this}}
    , m_progress {new QProgressBar {this}}
    , m_files {new QTreeWidget {this}}
    , m_consent {new QCheckBox {tr("I authorize this operation and have closed other programs that can write to the target."), this}}
{
    setWindowTitle(tr("Smart repair"));
    setWindowModality(Qt::WindowModal);
    resize(880, 620);

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
    details->addRow(tr("Target destination:"), location);
    auto *mode = new QComboBox {this};
    mode->addItems({tr("Safe staged update"), tr("Repair in place, without rollback"), tr("Recover interrupted staged update")});
    if (QFileInfo::exists(BitTorrent::StagingOperation::journalPath(torrent->id().toString())))
        mode->setCurrentIndex(2);
    details->addRow(tr("Operation:"), mode);
    auto *roots = new QPlainTextEdit {this};
    roots->setPlaceholderText(tr("Optional source directories, one per line. Only these locations are searched."));
    roots->setMaximumHeight(64);
    details->addRow(tr("Find existing data:"), roots);
    auto *browse = new QPushButton {tr("Add source directory…"), this};
    details->addRow(QString {}, browse);
    auto *chooseSource = new QPushButton {tr("Choose source for selected target file…"), this};
    details->addRow(QString {}, chooseSource);
    connect(chooseSource, &QPushButton::clicked, this, [this]
    {
        QTreeWidgetItem *item = m_files->currentItem();
        if (!item)
            return;
        const QString source = QFileDialog::getOpenFileName(this, tr("Choose existing bytes for this target file"));
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
    layout->addLayout(details);

    m_status->setTextFormat(Qt::PlainText);
    m_status->setTextInteractionFlags(Qt::TextSelectableByMouse);
    m_status->setWordWrap(true);
    m_status->setText(tr("Choose an operation and analyze the target torrent. Analysis does not change source data."));
    layout->addWidget(m_status);

    m_progress->setRange(0, 0);
    m_progress->setTextVisible(false);
    m_progress->hide();
    layout->addWidget(m_progress);

    m_files->setHeaderLabels({tr("File"), tr("Expected bytes"), tr("Actual bytes"), tr("Verified bytes"), tr("Problems")});
    m_files->setRootIsDecorated(false);
    m_files->setAlternatingRowColors(true);
    m_files->setSelectionMode(QAbstractItemView::ExtendedSelection);
    m_files->setEditTriggers(QAbstractItemView::NoEditTriggers);
    m_files->header()->setSectionResizeMode(0, QHeaderView::Stretch);
    for (int column = 1; column <= 3; ++column)
        m_files->header()->setSectionResizeMode(column, QHeaderView::ResizeToContents);
    layout->addWidget(m_files, 1);
    for (int index = 0; index < torrent->filesCount(); ++index)
    {
        auto *item = new QTreeWidgetItem {m_files, {torrent->filePath(index).toString()
            , locale().toString(torrent->fileSize(index)), tr("Not analyzed"), QString {}, QString {}}};
        item->setData(0, Qt::UserRole, int(torrent->info().nativeIndexes().at(index)));
    }

    auto *warning = new QLabel {tr("Safe update creates an independent target layout in staging and downloads selected missing data. Originals remain until explicit commit. "
        "Commit replaces only selected torrent files, retaining originals as recoverable backups. "
        "Repair in place writes directly and has no rollback. Files absent from the torrent are always preserved."), this};
    warning->setWordWrap(true);
    layout->addWidget(warning);
    m_consent->setEnabled(false);
    layout->addWidget(m_consent);

    auto *buttons = new QDialogButtonBox {QDialogButtonBox::Close, this};
    auto *analyze = buttons->addButton(tr("Analyze"), QDialogButtonBox::ActionRole);
    m_apply = buttons->addButton(tr("Prepare staging"), QDialogButtonBox::ActionRole);
    m_commit = buttons->addButton(tr("Commit verified update"), QDialogButtonBox::ActionRole);
    m_rollback = buttons->addButton(tr("Roll back"), QDialogButtonBox::ActionRole);
    m_commit->setEnabled(false);
    m_rollback->setEnabled(false);
    m_apply->setEnabled(false);
    m_apply->setAutoDefault(false);
    buttons->button(QDialogButtonBox::Close)->setDefault(true);
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
        m_apply->setEnabled(consent && (!m_staged || m_stagingStatus.value(QStringLiteral("can_prepare")).toBool()));
        m_commit->setEnabled(consent && m_stagingStatus.value(QStringLiteral("can_commit")).toBool());
        m_rollback->setEnabled(consent && m_stagingStatus.value(QStringLiteral("can_rollback")).toBool());
    });
    connect(analyze, &QPushButton::clicked, this, [this, mode, roots, analyze, updateSources]
    {
        mode->setEnabled(false);
        updateSources();
        analyze->setEnabled(false);
        m_progress->show();
        m_staged = mode->currentIndex() != 1;
        m_apply->setText(m_staged ? tr("Prepare / resume staging") : tr("Repair in place and recheck"));
        if (!m_staged)
        {
            m_consent->setText(tr("I closed other writers and consent to repair in place without rollback."));
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
        m_status->setText(m_staged ? tr("Copying candidates into independent staging, then checking and downloading with the torrent engine…")
            : tr("Preparing files for an in-place repair and starting the torrent recheck…"));
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
        m_status->setText(m_staged ? tr("Checking staging before downloading. Closing this dialog stops the operation and retains its recovery journal.")
            : tr("Checking the prepared data. Closing this dialog cancels the recheck."));
    });
    connect(m_service, &BitTorrent::RepairService::recheckFinished, this, [this]()
    {
        m_progress->hide();
        m_status->setText(tr("Recheck finished. The torrent is stopped. "
            "Close this dialog and use Start to download any missing data."));
    });

    connect(m_service, &BitTorrent::RepairService::stagingChanged, this, [this](const QJsonObject &status)
    {
        m_stagingStatus = status;
        const QString state = status.value(QStringLiteral("state")).toString();
        m_progress->setVisible((state == u"downloading") || (!status.value(QStringLiteral("finalized")).toBool()
            && !status.value(QStringLiteral("can_prepare")).toBool() && !status.value(QStringLiteral("can_commit")).toBool()
            && !status.value(QStringLiteral("can_rollback")).toBool()));
        m_consent->setChecked(false);
        m_consent->setEnabled(status.value(QStringLiteral("can_prepare")).toBool() || status.value(QStringLiteral("can_commit")).toBool()
            || status.value(QStringLiteral("can_rollback")).toBool());
        if ((state == u"planned") && status.value(QStringLiteral("can_prepare")).toBool())
        {
            m_status->setText(m_status->text() + tr("\nIndependent staging requires %L1 additional bytes; %L2 bytes are available.")
                .arg(status.value(QStringLiteral("required_bytes")).toString().toLongLong())
                .arg(status.value(QStringLiteral("available_bytes")).toString().toLongLong()));
        }
        else if (state == u"ready_to_commit")
        {
            m_status->setText(tr("Every selected target hash and exact file size is verified. Close other writers and confirm commit, or roll back."));
        }
        else if ((state == u"committed") && status.value(QStringLiteral("finalized")).toBool())
        {
            m_status->setText(tr("Verified update committed. The torrent is stopped at the destination. Original files remain in the staging backup directory."));
        }
        else if ((state == u"rolled_back") && status.value(QStringLiteral("finalized")).toBool())
        {
            m_status->setText(tr("Rollback completed. Original target files are restored; independent staged data and unknown files are preserved."));
        }
        else
        {
            m_status->setText(tr("Staged operation: %1. Recovery data is retained at %2.")
                .arg(state, status.value(QStringLiteral("payload_path")).toString()));
        }
    });
    connect(m_commit, &QPushButton::clicked, this, [this]
    {
        m_commit->setEnabled(false);
        m_rollback->setEnabled(false);
        m_consent->setEnabled(false);
        m_service->commitStaged();
    });
    connect(m_rollback, &QPushButton::clicked, this, [this]
    {
        m_commit->setEnabled(false);
        m_rollback->setEnabled(false);
        m_consent->setEnabled(false);
        m_service->rollbackStaged();
    });
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
        for (int column = 1; column <= 3; ++column)
            item->setTextAlignment(column, Qt::AlignRight | Qt::AlignVCenter);
    }

    QString summary = tr("Verified %L1 of %L2 bytes. %L3 valid pieces; %L4 unverified pieces.")
        .arg(analysis.verifiedBytes).arg(analysis.expectedBytes).arg(analysis.validPieces).arg(analysis.unverifiedPieces);
    if (analysis.wholeFileV2Verification)
        summary += tr("\nVerification used v2 whole-file roots; partial files require the torrent recheck.");
    m_status->setText(summary);
    m_consent->setEnabled(true);
}

void RepairDialog::showFailure(const QString &message)
{
    m_progress->hide();
    m_consent->setChecked(false);
    m_consent->setEnabled(false);
    m_apply->setEnabled(false);
    m_status->setText(tr("Repair cannot continue: %1").arg(message));
}
