/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "releaseupdatedialog.h"

#include <QDesktopServices>
#include <QDialogButtonBox>
#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QLabel>
#include <QProgressBar>
#include <QPushButton>
#include <QStandardPaths>
#include <QVBoxLayout>

#include "base/global.h"
#include "base/version.h"

ReleaseUpdateDialog::ReleaseUpdateDialog(QWidget *parent)
    : QDialog(parent)
    , m_updater(this)
{
    setWindowTitle(tr("qbutt updates"));
    setAttribute(Qt::WA_DeleteOnClose);
    setMinimumWidth(520);
    auto *layout = new QVBoxLayout(this);
    layout->addWidget(new QLabel(tr("Installed: qbutt %1")
        .arg(QStringLiteral(QBUTT_VERSION)), this));
    auto *source = new QLabel(tr("Source: <a href=\"https://github.com/qbutt-org/qbutt/releases\">qbutt releases</a>"), this);
    source->setOpenExternalLinks(true);
    source->setToolTip(tr("Updates use the system connection."));
    layout->addWidget(source);
    m_status = new QLabel(this);
    m_status->setObjectName(u"releaseStatus"_s);
    m_status->setTextFormat(Qt::PlainText);
    m_status->setWordWrap(true);
    m_status->setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Minimum);
    layout->addWidget(m_status);
    m_progress = new QProgressBar(this);
    m_progress->setObjectName(u"releaseProgress"_s);
    layout->addWidget(m_progress);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Close, this);
    m_check = buttons->addButton(tr("Check for updates"), QDialogButtonBox::ActionRole);
    m_check->setObjectName(u"releaseCheck"_s);
    m_download = buttons->addButton(tr("Download ZIP…"), QDialogButtonBox::ActionRole);
    m_download->setObjectName(u"releaseDownload"_s);
    m_cancel = buttons->addButton(tr("Cancel download"), QDialogButtonBox::ActionRole);
    m_openFolder = buttons->addButton(tr("Open folder"), QDialogButtonBox::ActionRole);
    layout->addWidget(buttons);
    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
    connect(this, &QDialog::finished, &m_updater, &ReleaseUpdater::cancel);
    connect(m_check, &QPushButton::clicked, &m_updater, &ReleaseUpdater::check);
    connect(m_cancel, &QPushButton::clicked, &m_updater, &ReleaseUpdater::cancel);
    connect(m_download, &QPushButton::clicked, this, [this]()
    {
        const QString path = QFileDialog::getSaveFileName(this, tr("Save qbutt update"),
            QDir(QStandardPaths::writableLocation(QStandardPaths::DownloadLocation)).filePath(m_updater.fileName()),
            tr("ZIP archives (*.zip)"));
        if (!path.isEmpty())
            m_updater.download(path);
    });
    connect(m_openFolder, &QPushButton::clicked, this, [this]()
    {
        QDesktopServices::openUrl(QUrl::fromLocalFile(QFileInfo(m_updater.savedPath()).absolutePath()));
    });
    connect(&m_updater, &ReleaseUpdater::changed, this, &ReleaseUpdateDialog::refresh);
    connect(&m_updater, &ReleaseUpdater::progress, this, [this](const qint64 received, const qint64 total)
    {
        m_progress->setRange(0, 1000);
        m_progress->setValue(static_cast<int>(received * 1000 / total));
    });
    m_updater.check();
}

void ReleaseUpdateDialog::refresh()
{
    const auto state = m_updater.state();
    const bool busy = (state == ReleaseUpdater::State::Checking) || (state == ReleaseUpdater::State::Downloading);
    m_status->setText(m_updater.message());
    m_check->setEnabled(!busy);
    m_download->setEnabled(state == ReleaseUpdater::State::Available);
    m_cancel->setVisible(busy);
    m_cancel->setText(state == ReleaseUpdater::State::Checking ? tr("Cancel check") : tr("Cancel download"));
    m_openFolder->setVisible(state == ReleaseUpdater::State::Ready);
    m_progress->setVisible(busy);
    m_progress->setRange(0, 0);
}
