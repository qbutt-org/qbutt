// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QHash>
#include <QJsonObject>
#include <QPointer>
#include <QSet>
#include <QUrl>
#include <QWidget>

#include "base/bittorrent/infohash.h"
#include "gui/guiapplicationcomponent.h"

class QCheckBox;
class QComboBox;
class QJsonArray;
class QLabel;
class QLineEdit;
class QPushButton;
class QSplitter;
class QStackedWidget;
class QStandardItemModel;
class QTabBar;
class QTableView;
class QTabWidget;
class QTextBrowser;
class QToolButton;

namespace BitTorrent
{
    class Torrent;
}

namespace Content
{
    class DownloadsWidget;
    class ReaderWidget;

    class LibraryWidget final : public GUIApplicationComponent<QWidget>
    {
        Q_OBJECT

    public:
        explicit LibraryWidget(IGUIApplication *app, QWidget *parent = nullptr);
        ~LibraryWidget() override;
        void giveFocusToSearchInput();
        void acceptSharedLink(const QUrl &url);
        void downloadMedia(const QJsonObject &params);
        bool openLocalFile();
        bool isReading() const;

    signals:
        void searchFinished(bool failed);
        void readerActiveChanged(bool active);

    private:
        class VariantFilterModel;

        void search(bool expanded = false);
        void changeView();
        void loadLibrary();
        QJsonObject libraryEntry(const QJsonObject &work) const;
        void clearDetails();
        void upsertWork(QJsonObject work);
        void selectWork();
        void showWork(const QJsonObject &work, const QJsonObject &preferredVariant = {}, bool revealPreferred = false);
        void updateVariantFacets(const QJsonArray &variants, const QJsonObject &preferredVariant);
        void applyVariantFilters();
        void filterVariants();
        void updateVariantColumns();
        void selectVariant();
        void inspectVariant();
        void resolveVariant(bool downloadOnly = false);
        void startResolve();
        void downloadVariant();
        ReaderWidget *ensureReader();
        void openResolved(const QJsonObject &resolved);
        void watchTorrent(BitTorrent::Torrent *torrent);
        void playTorrentFile(BitTorrent::Torrent *torrent, int fileIndex, const QJsonObject &playback);
        void clearTorrentWatch();
        void saveWork();
        void shareWork(const QString &mode);
        void showSources();
        void handleEvent(qint64 id, const QString &event, const QJsonValue &data);
        QJsonObject currentVariant() const;
        void updateActions();

        QWidget *m_navigation = nullptr;
        QLineEdit *m_query = nullptr;
        QComboBox *m_category = nullptr;
        QPushButton *m_search = nullptr;
        QPushButton *m_expand = nullptr;
        QPushButton *m_openSource = nullptr;
        QPushButton *m_retry = nullptr;
        QTabBar *m_views = nullptr;
        QLabel *m_status = nullptr;
        QTableView *m_works = nullptr;
        DownloadsWidget *m_downloads = nullptr;
        QStandardItemModel *m_searchModel = nullptr;
        QStandardItemModel *m_libraryModel = nullptr;
        QHash<QString, int> m_workRows;
        QSplitter *m_splitter = nullptr;
        QStackedWidget *m_pages = nullptr;
        QWidget *m_details = nullptr;
        QLabel *m_title = nullptr;
        QTabWidget *m_detailPages = nullptr;
        QTableView *m_variants = nullptr;
        QLineEdit *m_variantFilter = nullptr;
        QWidget *m_variantFacets = nullptr;
        QComboBox *m_episode = nullptr;
        QComboBox *m_voice = nullptr;
        QComboBox *m_source = nullptr;
        QCheckBox *m_showReported = nullptr;
        QStandardItemModel *m_variantModel = nullptr;
        VariantFilterModel *m_variantProxy = nullptr;
        QTextBrowser *m_notes = nullptr;
        QPushButton *m_open = nullptr;
        QPushButton *m_download = nullptr;
        QPushButton *m_save = nullptr;
        QToolButton *m_share = nullptr;
        QPushButton *m_remove = nullptr;
        QPointer<ReaderWidget> m_reader;
        QJsonObject m_work;
        QJsonObject m_inspectingVariant;
        QJsonObject m_resolvingWork;
        QJsonObject m_resolvingVariant;
        QJsonObject m_playback;
        QUrl m_failureUrl;
        QHash<QString, QJsonObject> m_libraryEntries;
        QSet<qint64> m_rememberRequests;
        QPointer<BitTorrent::Torrent> m_watchTorrent;
        BitTorrent::InfoHash m_watchInfoHash;
        QJsonObject m_torrentPlayback;
        bool m_watchResolvedTorrent = false;
        qint64 m_searchRequest = 0;
        qint64 m_libraryRequest = 0;
        qint64 m_detailsRequest = 0;
        qint64 m_resolveRequest = 0;
        qint64 m_shareRequest = 0;
        qint64 m_linkRequest = 0;
        qint64 m_removeRequest = 0;
        bool m_searchFailed = false;
        bool m_expandedSearch = false;
    };
}
