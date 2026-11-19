/*
 * Bittorrent Client using Qt and libtorrent.
 * Copyright (C) 2022-2025  Vladimir Golovnev <glassez@yandex.ru>
 * Copyright (C) 2012  Christophe Dumez <chris@qbittorrent.org>
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public License
 * as published by the Free Software Foundation; either version 2
 * of the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program; if not, write to the Free Software
 * Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA  02110-1301, USA.
 *
 * In addition, as a special exception, the copyright holders give permission to
 * link this program with the OpenSSL project's "OpenSSL" library (or with
 * modified versions of it that use the same license as the "OpenSSL" library),
 * and distribute the linked executables. You must obey the GNU General Public
 * License in all respects for all of the code used other than "OpenSSL".  If you
 * modify file(s), you may extend this exception to your version of the file(s),
 * but you are not obligated to do so. If you do not wish to do so, delete this
 * exception statement from your version.
 */

#include "addnewtorrentdialog.h"

#include <algorithm>
#include <functional>

#include <QtVersionChecks>
#include <QAction>
#include <QByteArray>
#include <QFuture>
#include <QHeaderView>
#include <QList>
#include <QMenu>
#include <QPushButton>
#include <QShortcut>
#include <QSize>
#include <QString>

#include "base/bittorrent/addtorrentparams.h"
#include "base/bittorrent/downloadpriority.h"
#include "base/bittorrent/infohash.h"
#include "base/bittorrent/session.h"
#include "base/bittorrent/torrent.h"
#include "base/bittorrent/torrentcontenthandler.h"
#include "base/bittorrent/torrentcontentlayout.h"
#include "base/bittorrent/torrentdescriptor.h"
#include "base/global.h"
#include "base/preferences.h"
#include "base/settingsstorage.h"
#include "base/torrentfileguard.h"
#include "filterpatternformatmenu.h"
#include "lineedit.h"

#include "ui_addnewtorrentdialog.h"

namespace
{
#define SETTINGS_KEY(name) u"AddNewTorrentDialog/" name
    const QString KEY_SAVEPATHHISTORY = SETTINGS_KEY(u"SavePathHistory"_s);

    // just a shortcut
    inline SettingsStorage *settings()
    {
        return SettingsStorage::instance();
    }

    // savePath is a folder, not an absolute file path
    int indexOfPath(const FileSystemPathComboEdit *fsPathEdit, const Path &savePath)
    {
        for (qsizetype i = 0; i < fsPathEdit->count(); ++i)
        {
            if (fsPathEdit->item(i) == savePath)
                return i;
        }
        return -1;
    }

    void setPath(FileSystemPathComboEdit *fsPathEdit, const Path &newPath)
    {
        int existingIndex = indexOfPath(fsPathEdit, newPath);
        if (existingIndex < 0)
        {
            // New path, prepend to combo box
            fsPathEdit->insertItem(0, newPath);
            existingIndex = 0;
        }

        fsPathEdit->setCurrentIndex(existingIndex);
    }

    void rememberSavePath(const Path &path)
    {
        auto pathList = settings()->loadValue<QStringList>(KEY_SAVEPATHHISTORY);

        const qsizetype selectedSavePathIndex = pathList.indexOf(path.toString());
        if (selectedSavePathIndex > -1)
            pathList.move(selectedSavePathIndex, 0);
        else
            pathList.prepend(path.toString());

        const int maxLength = Preferences::instance()->addNewTorrentDialogSavePathHistoryLength();
        if (pathList.size() > maxLength)
            pathList.resize(maxLength);

        settings()->storeValue(KEY_SAVEPATHHISTORY, pathList);
    }
}

class AddNewTorrentDialog::TorrentContentAdaptor final
        : public BitTorrent::TorrentContentHandler
{
public:
    TorrentContentAdaptor(const BitTorrent::TorrentInfo &torrentInfo, PathList &filePaths
            , QList<BitTorrent::DownloadPriority> &filePriorities, std::function<void ()> onFilePrioritiesChanged)
        : m_torrentInfo {torrentInfo}
        , m_filePaths {filePaths}
        , m_filePriorities {filePriorities}
        , m_onFilePrioritiesChanged {std::move(onFilePrioritiesChanged)}
    {
        Q_ASSERT(filePaths.isEmpty() || (filePaths.size() == m_torrentInfo.filesCount()));

        m_originalRootFolder = Path::findRootFolder(m_torrentInfo.filePaths());
        m_currentContentLayout = (m_originalRootFolder.isEmpty()
                ? BitTorrent::TorrentContentLayout::NoSubfolder
                : BitTorrent::TorrentContentLayout::Subfolder);

        if (const int fileCount = filesCount(); !m_filePriorities.isEmpty() && (fileCount >= 0))
            m_filePriorities.resize(fileCount, BitTorrent::DownloadPriority::Normal);
    }

    bool hasMetadata() const override
    {
        return m_torrentInfo.isValid();
    }

    int filesCount() const override
    {
        return m_torrentInfo.filesCount();
    }

    qlonglong fileSize(const int index) const override
    {
        Q_ASSERT((index >= 0) && (index < filesCount()));
        return m_torrentInfo.fileSize(index);
    }

    Path filePath(const int index) const override
    {
        Q_ASSERT((index >= 0) && (index < filesCount()));
        return (m_filePaths.isEmpty() ? m_torrentInfo.filePath(index) : m_filePaths.at(index));
    }

    PathList filePaths() const
    {
        return (m_filePaths.isEmpty() ? m_torrentInfo.filePaths() : m_filePaths);
    }

    void renameFile(const int index, const Path &newFilePath) override
    {
        Q_ASSERT((index >= 0) && (index < filesCount()));
        const Path currentFilePath = filePath(index);
        if (currentFilePath == newFilePath)
            return;

        if (m_filePaths.isEmpty())
            m_filePaths = m_torrentInfo.filePaths();

        m_filePaths[index] = newFilePath;
    }

    void applyContentLayout(const BitTorrent::TorrentContentLayout contentLayout)
    {
        Q_ASSERT(hasMetadata());
        Q_ASSERT(!m_filePaths.isEmpty());

        const auto originalContentLayout = (m_originalRootFolder.isEmpty()
                ? BitTorrent::TorrentContentLayout::NoSubfolder
                : BitTorrent::TorrentContentLayout::Subfolder);
        const auto newContentLayout = ((contentLayout == BitTorrent::TorrentContentLayout::Original)
                ? originalContentLayout : contentLayout);
        if (newContentLayout != m_currentContentLayout)
        {
            if (newContentLayout == BitTorrent::TorrentContentLayout::NoSubfolder)
            {
                Path::stripRootFolder(m_filePaths);
            }
            else
            {
                const auto rootFolder = ((originalContentLayout == BitTorrent::TorrentContentLayout::Subfolder)
                        ? m_originalRootFolder : m_filePaths.at(0).removedExtension());
                Path::addRootFolder(m_filePaths, rootFolder);
            }

            m_currentContentLayout = newContentLayout;
        }
    }

    QList<BitTorrent::DownloadPriority> filePriorities() const override
    {
        return m_filePriorities.isEmpty()
                ? QList<BitTorrent::DownloadPriority>(filesCount(), BitTorrent::DownloadPriority::Normal)
                : m_filePriorities;
    }

    QList<qreal> filesProgress() const override
    {
        return QList<qreal>(filesCount(), 0);
    }

    QFuture<QList<qreal>> fetchAvailableFileFractions() const override
    {
#if (QT_VERSION >= QT_VERSION_CHECK(6, 6, 0))
        return QtFuture::makeReadyValueFuture(QList<qreal>(filesCount(), 0));
#else
        return QtFuture::makeReadyFuture(QList<qreal>(filesCount(), 0));
#endif
    }

    void prioritizeFiles(const QList<BitTorrent::DownloadPriority> &priorities) override
    {
        Q_ASSERT(priorities.size() == filesCount());
        m_filePriorities = priorities;
        if (m_onFilePrioritiesChanged)
            m_onFilePrioritiesChanged();
    }

    Path actualStorageLocation() const override
    {
        return {};
    }

    Path actualFilePath([[maybe_unused]] int fileIndex) const override
    {
        return {};
    }

    void flushCache() const override
    {
    }

private:
    const BitTorrent::TorrentInfo &m_torrentInfo;
    PathList &m_filePaths;
    QList<BitTorrent::DownloadPriority> &m_filePriorities;
    std::function<void ()> m_onFilePrioritiesChanged;
    Path m_originalRootFolder;
    BitTorrent::TorrentContentLayout m_currentContentLayout;
};

AddNewTorrentDialog::AddNewTorrentDialog(const BitTorrent::TorrentDescriptor &torrentDescr
        , const BitTorrent::AddTorrentParams &inParams, QWidget *parent)
    : QDialog(parent)
    , m_torrentDescr {torrentDescr}
    , m_torrentParams {inParams}
    , m_ui {new Ui::AddNewTorrentDialog}
    , m_storeDialogSize {SETTINGS_KEY(u"DialogSize"_s)}
    , m_storeTreeHeaderState {u"GUI/Qt6/" SETTINGS_KEY(u"TreeHeaderState"_s)}
    , m_storeFilterPatternFormat {u"GUI/" SETTINGS_KEY(u"FilterPatternFormat"_s)}
{
    m_ui->setupUi(this);

    m_ui->savePath->setMode(FileSystemPathEdit::Mode::DirectorySave);
    m_ui->savePath->setDialogCaption(tr("Choose save path"));
    m_ui->savePath->setMaxVisibleItems(20);

    m_ui->doNotDeleteTorrentCheckBox->setVisible(TorrentFileGuard::autoDeleteMode() != TorrentFileGuard::Never);

    // Torrent content filtering
    m_ui->filterFiles->setPlaceholderText(tr("Filter files..."));
    m_ui->filterFiles->setContextMenuPolicy(Qt::CustomContextMenu);
    connect(m_ui->filterFiles, &QWidget::customContextMenuRequested, this, &AddNewTorrentDialog::showContentFilterContextMenu);
    const auto *focusSearchHotkey = new QShortcut(QKeySequence::Find, this);
    connect(focusSearchHotkey, &QShortcut::activated, this, [this]()
    {
        m_ui->filterFiles->setFocus();
        m_ui->filterFiles->selectAll();
    });

    if (const QSize dialogSize = m_storeDialogSize; dialogSize.isValid())
        resize(dialogSize);

    QHeaderView *const contentHeader = m_ui->contentTreeView->header();
    if (const QByteArray state = m_storeTreeHeaderState; state.isEmpty() || !contentHeader->restoreState(state))
    {
        contentHeader->setStretchLastSection(false);
        contentHeader->setSectionResizeMode(TorrentContentWidget::Name, QHeaderView::Stretch);
        m_ui->contentTreeView->setColumnWidth(TorrentContentWidget::Priority,
            std::max(110, contentHeader->sectionSizeHint(TorrentContentWidget::Priority)));
    }
    // Hide useless columns after loading the header state
    m_ui->contentTreeView->hideColumn(TorrentContentWidget::Progress);
    m_ui->contentTreeView->hideColumn(TorrentContentWidget::Remaining);
    m_ui->contentTreeView->hideColumn(TorrentContentWidget::Availability);
    m_ui->contentTreeView->setColumnsVisibilityMode(TorrentContentWidget::ColumnsVisibilityMode::Locked);
    m_ui->contentTreeView->setDoubleClickAction(TorrentContentWidget::DoubleClickAction::Rename);

    connect(m_ui->buttonBox, &QDialogButtonBox::accepted, this, &QDialog::accept);
    connect(m_ui->buttonBox, &QDialogButtonBox::rejected, this, &QDialog::reject);
    connect(m_ui->filterFiles, &LineEdit::textChanged, this, &AddNewTorrentDialog::setContentFilterPattern);
    connect(m_ui->toggleFiles, &QCheckBox::clicked, this, [this]
    {
        auto priorities = m_contentAdaptor->filePriorities();
        if (priorities.contains(BitTorrent::DownloadPriority::Ignored))
            std::ranges::replace(priorities, BitTorrent::DownloadPriority::Ignored, BitTorrent::DownloadPriority::Normal);
        else
            priorities.fill(BitTorrent::DownloadPriority::Ignored);
        m_contentAdaptor->prioritizeFiles(priorities);
        m_ui->contentTreeView->refresh();
    });
    connect(Preferences::instance(), &Preferences::changed, this, []
    {
        const int length = Preferences::instance()->addNewTorrentDialogSavePathHistoryLength();
        settings()->storeValue(KEY_SAVEPATHHISTORY, settings()->loadValue<QStringList>(KEY_SAVEPATHHISTORY).mid(0, length));
    });

    const auto *session = BitTorrent::Session::instance();
    const auto history = settings()->loadValue<QStringList>(KEY_SAVEPATHHISTORY);
    for (const QString &path : history)
        m_ui->savePath->addItem(Path(path));

    const bool automatic = inParams.useAutoTMM.value_or(inParams.savePath.isEmpty()
        && inParams.downloadPath.isEmpty() && !session->isAutoTMMDisabledByDefault());
    m_torrentParams.useAutoTMM = automatic;
    const Path defaultPath = session->suggestedSavePath(inParams.category, inParams.useAutoTMM);
    if (!automatic && !inParams.savePath.isEmpty())
        setPath(m_ui->savePath, inParams.savePath.isAbsolute() ? inParams.savePath : defaultPath / inParams.savePath);
    else if (history.isEmpty() || !inParams.category.isEmpty() || inParams.useAutoTMM.value_or(false))
        setPath(m_ui->savePath, defaultPath);
    else
        m_ui->savePath->setCurrentIndex(0);

    m_ui->savePath->setFocus();
    const bool hasMetadata = torrentDescr.info().has_value();
    m_ui->lblMetaLoading->setVisible(!hasMetadata);
    m_ui->progMetaLoading->setVisible(!hasMetadata);
    m_ui->lblMetaLoading->setText(tr("Retrieving metadata..."));
    m_ui->toggleFiles->setEnabled(hasMetadata);
    m_ui->filterFiles->setEnabled(hasMetadata);
    if (hasMetadata)
        setupTreeview();
    else
        setWindowTitle(torrentDescr.name().isEmpty() ? tr("Magnet link") : torrentDescr.name());
}

AddNewTorrentDialog::~AddNewTorrentDialog()
{
    delete m_ui;
}

bool AddNewTorrentDialog::isDoNotDeleteTorrentChecked() const
{
    return m_ui->doNotDeleteTorrentCheckBox->isChecked();
}

void AddNewTorrentDialog::showEvent(QShowEvent *event)
{
    QDialog::showEvent(event);
    if (!Preferences::instance()->isAddNewTorrentDialogTopLevel())
        return;

    activateWindow();
    raise();
}

void AddNewTorrentDialog::showContentFilterContextMenu()
{
    QMenu *menu = m_ui->filterFiles->createStandardContextMenu();

    auto *formatMenu = new FilterPatternFormatMenu(m_storeFilterPatternFormat.get(FilterPatternFormat::Wildcards), menu);
    connect(formatMenu, &FilterPatternFormatMenu::patternFormatChanged, this, [this](const FilterPatternFormat format)
    {
        m_storeFilterPatternFormat = format;
        setContentFilterPattern();
    });

    menu->addSeparator();
    menu->addMenu(formatMenu);
    menu->setAttribute(Qt::WA_DeleteOnClose);
    menu->popup(QCursor::pos());
}

void AddNewTorrentDialog::setContentFilterPattern()
{
    m_ui->contentTreeView->setFilterPattern(m_ui->filterFiles->text(), m_storeFilterPatternFormat.get(FilterPatternFormat::Wildcards));
}

void AddNewTorrentDialog::accept()
{
    const auto *session = BitTorrent::Session::instance();
    const Path destination = m_ui->savePath->selectedPath();
    if (!m_torrentParams.useAutoTMM.value()
            || (destination != session->categorySavePath(m_torrentParams.category)))
    {
        m_torrentParams.useAutoTMM = false;
        m_torrentParams.savePath = destination;
    }
    rememberSavePath(destination);
    emit torrentAccepted(m_torrentDescr, m_torrentParams);
    QDialog::accept();
}

void AddNewTorrentDialog::reject()
{
    emit torrentRejected(m_torrentDescr);
    if (!m_torrentDescr.info())
        BitTorrent::Session::instance()->cancelDownloadMetadata(m_torrentDescr.infoHash().toTorrentID());

    QDialog::reject();
}

void AddNewTorrentDialog::done(const int result)
{
    m_storeDialogSize = size();
    if (m_torrentDescr.info())
        m_storeTreeHeaderState = m_ui->contentTreeView->header()->saveState();
    QDialog::done(result);
}

void AddNewTorrentDialog::updateMetadata(const BitTorrent::TorrentInfo &metadata)
{
    if (!metadata.isValid() || !metadata.matchesInfoHash(m_torrentDescr.infoHash()))
        return;

    m_torrentDescr.setTorrentInfo(metadata);
    setupTreeview();
    m_ui->lblMetaLoading->hide();
    m_ui->progMetaLoading->hide();
    m_ui->toggleFiles->setEnabled(true);
    m_ui->filterFiles->setEnabled(true);
}

void AddNewTorrentDialog::setupTreeview()
{
    setWindowTitle(m_torrentDescr.name());
    const auto &torrentInfo = *m_torrentDescr.info();
    auto &addTorrentParams = m_torrentParams;
    const auto *session = BitTorrent::Session::instance();
    if (addTorrentParams.stopCondition.value_or(session->torrentStopCondition()) == BitTorrent::Torrent::StopCondition::MetadataReceived)
    {
        addTorrentParams.addStopped = true;
        addTorrentParams.stopCondition = BitTorrent::Torrent::StopCondition::None;
    }

    if (addTorrentParams.filePaths.isEmpty())
        addTorrentParams.filePaths = torrentInfo.filePaths();

    m_contentAdaptor = std::make_unique<TorrentContentAdaptor>(torrentInfo, addTorrentParams.filePaths
            , addTorrentParams.filePriorities, [this] { updateSelectionAction(); });

    m_contentAdaptor->applyContentLayout(addTorrentParams.contentLayout.value_or(session->torrentContentLayout()));

    if (BitTorrent::Session::instance()->isExcludedFileNamesEnabled())
    {
        // Check file name blacklist for torrents that are manually added
        QList<BitTorrent::DownloadPriority> priorities = m_contentAdaptor->filePriorities();
        BitTorrent::Session::instance()->applyFilenameFilter(m_contentAdaptor->filePaths(), priorities);
        m_contentAdaptor->prioritizeFiles(priorities);
    }

    m_ui->contentTreeView->setContentHandler(m_contentAdaptor.get());

    updateSelectionAction();
}

void AddNewTorrentDialog::updateSelectionAction()
{
    const auto priorities = m_contentAdaptor->filePriorities();
    const auto ignored = priorities.count(BitTorrent::DownloadPriority::Ignored);
    const auto state = (ignored == 0) ? Qt::Checked
            : (ignored == priorities.size()) ? Qt::Unchecked : Qt::PartiallyChecked;
    m_ui->toggleFiles->setCheckState(state);
}
