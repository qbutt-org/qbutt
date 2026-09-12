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
#include <QDialogButtonBox>
#include <QDir>
#include <QFormLayout>
#include <QHeaderView>
#include <QLabel>
#include <QLocale>
#include <QProgressBar>
#include <QPushButton>
#include <QTreeWidget>
#include <QVBoxLayout>

#include "base/bittorrent/repairanalysis.h"
#include "base/bittorrent/repairservice.h"
#include "base/bittorrent/torrent.h"
#include "base/path.h"

RepairDialog::RepairDialog(QWidget *parent, BitTorrent::Torrent *torrent)
    : QDialog {parent}
    , m_service {new BitTorrent::RepairService {torrent, this}}
    , m_status {new QLabel {this}}
    , m_progress {new QProgressBar {this}}
    , m_files {new QTreeWidget {this}}
    , m_consent {new QCheckBox {tr("I closed other writers and consent to repair in place without rollback."), this}}
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
    auto *location = new QLabel {torrent->actualStorageLocation().toString(), this};
    location->setTextFormat(Qt::PlainText);
    location->setTextInteractionFlags(Qt::TextSelectableByMouse);
    location->setWordWrap(true);
    details->addRow(tr("Current location:"), location);
    layout->addLayout(details);

    m_status->setTextFormat(Qt::PlainText);
    m_status->setTextInteractionFlags(Qt::TextSelectableByMouse);
    m_status->setWordWrap(true);
    m_status->setText(tr("Analyzing existing files using this torrent's current file mappings. No data is changed during analysis."));
    layout->addWidget(m_status);

    m_progress->setRange(0, 0);
    m_progress->setTextVisible(false);
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

    auto *warning = new QLabel {tr("Repair in place can replace existing data and truncates oversized files to the torrent's exact sizes. "
        "Size changes use exclusive file access, then the standard torrent engine takes over for recheck. "
        "There is no rollback. Keep other applications that can write to this location closed throughout repair and downloading. "
        "Files not listed in this torrent are left untouched."), this};
    warning->setWordWrap(true);
    layout->addWidget(warning);
    m_consent->setEnabled(false);
    layout->addWidget(m_consent);

    auto *buttons = new QDialogButtonBox {QDialogButtonBox::Close, this};
    m_apply = buttons->addButton(tr("Repair in place and recheck"), QDialogButtonBox::ActionRole);
    m_apply->setEnabled(false);
    m_apply->setAutoDefault(false);
    buttons->button(QDialogButtonBox::Close)->setDefault(true);
    layout->addWidget(buttons);

    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
    connect(m_consent, &QCheckBox::toggled, m_apply, &QPushButton::setEnabled);
    connect(m_apply, &QPushButton::clicked, this, [this]()
    {
        m_apply->setEnabled(false);
        m_consent->setEnabled(false);
        m_progress->show();
        m_status->setText(tr("Preparing files for an in-place repair and starting the torrent recheck..."));
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
        m_status->setText(tr("Checking the prepared data. Closing this dialog cancels the recheck."));
    });
    connect(m_service, &BitTorrent::RepairService::recheckFinished, this, [this]()
    {
        m_progress->hide();
        m_status->setText(tr("Recheck finished. The torrent is stopped. "
            "Close this dialog and use Start to download any missing data."));
    });

    m_service->analyze();
}

void RepairDialog::showAnalysis(const BitTorrent::RepairAnalysis &analysis, const QString &directory)
{
    m_progress->hide();
    for (const BitTorrent::RepairFileAnalysis &file : analysis.files)
    {
        auto *item = new QTreeWidgetItem {m_files, {QDir(directory).relativeFilePath(file.path), locale().toString(file.expectedSize)
            , (file.actualSize < 0) ? tr("Missing") : locale().toString(file.actualSize)
            , locale().toString(file.verifiedBytes), file.problems.join(u'\n')}};
        item->setToolTip(0, file.path);
        item->setToolTip(4, file.problems.join(u'\n'));
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
