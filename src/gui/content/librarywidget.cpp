// SPDX-License-Identifier: GPL-2.0-or-later
#include "librarywidget.h"

#include <algorithm>
#include <utility>

#include <QApplication>
#include <QCheckBox>
#include <QClipboard>
#include <QCollator>
#include <QComboBox>
#include <QDesktopServices>
#include <QGuiApplication>
#include <QHash>
#include <QHeaderView>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QJsonArray>
#include <QLabel>
#include <QLineEdit>
#include <QLocale>
#include <QMenu>
#include <QPainter>
#include <QProgressBar>
#include <QPushButton>
#include <QRegularExpression>
#include <QSet>
#include <QSignalBlocker>
#include <QSortFilterProxyModel>
#include <QSplitter>
#include <QStackedWidget>
#include <QStandardItemModel>
#include <QStyle>
#include <QStyledItemDelegate>
#include <QTabBar>
#include <QTableView>
#include <QTabWidget>
#include <QTextBrowser>
#include <QTextDocument>
#include <QTextLayout>
#include <QToolButton>
#include <QUrl>
#include <QVBoxLayout>

#include "base/bittorrent/addtorrenterror.h"
#include "base/bittorrent/session.h"
#include "base/bittorrent/torrent.h"
#include "base/bittorrent/torrentdescriptor.h"
#include "base/content/client.h"
#include "base/global.h"
#include "base/media/player.h"
#include "base/preferences.h"
#include "base/utils/misc.h"
#include "gui/flowlayout.h"
#include "gui/interfaces/iguiapplication.h"
#include "gui/uithememanager.h"
#include "browserprerequisitewidget.h"
#include "downloadswidget.h"
#include "readerwidget.h"
#include "sourcesdialog.h"

namespace
{
    class WorkTitleLabel final : public QLabel
    {
    public:
        using QLabel::QLabel;

        QSize sizeHint() const override
        {
            return fontMetrics().size(Qt::TextSingleLine, text());
        }
    };

    enum VariantColumns
    {
        VariantColumn,
        SourceColumn,
        SizeColumn,
        SeedsColumn
    };

    QString categoryName(const QString &category)
    {
        if (category == u"games")
            return Content::LibraryWidget::tr("Games");
        if (category == u"movies")
            return Content::LibraryWidget::tr("Movies");
        if (category == u"tv")
            return Content::LibraryWidget::tr("Series");
        if (category == u"anime")
            return Content::LibraryWidget::tr("Anime");
        if (category == u"software")
            return Content::LibraryWidget::tr("Software");
        if (category == u"books")
            return Content::LibraryWidget::tr("Books");
        if (category == u"manga")
            return Content::LibraryWidget::tr("Manga");
        return Content::LibraryWidget::tr("All");
    }

    void configureTable(QTableView *view)
    {
        view->setSelectionBehavior(QAbstractItemView::SelectRows);
        view->setSelectionMode(QAbstractItemView::SingleSelection);
        view->setEditTriggers(QAbstractItemView::NoEditTriggers);
        view->setShowGrid(false);
        view->setFrameShape(QFrame::NoFrame);
        view->setWordWrap(false);
        view->setAlternatingRowColors(false);
        view->verticalHeader()->hide();
        view->verticalHeader()->setDefaultSectionSize(34);
        view->horizontalHeader()->setHighlightSections(false);
        view->horizontalHeader()->setStretchLastSection(false);
    }

    QStringList workSources(const QJsonObject &work)
    {
        QStringList sources;
        QSet<QString> sourceIds;
        for (const QJsonValue &value : work.value(u"variants"_s).toArray())
        {
            const QJsonObject variant = value.toObject();
            const QString sourceId = variant.value(u"sourceId"_s).toString();
            if (!sourceIds.contains(sourceId))
            {
                sourceIds.insert(sourceId);
                sources.append(variant.value(u"sourceName"_s).toString(sourceId));
            }
        }
        return sources;
    }

    QString workTitle(const QJsonObject &work)
    {
        const QString title = work.value(u"title"_s).toString();
        const int year = work.value(u"year"_s).toInt();
        return year > 0 ? u"%1 (%2)"_s.arg(title).arg(year) : title;
    }

    QString workToolTip(const QJsonObject &work)
    {
        return workTitle(work) + u'\n' + categoryName(work.value(u"category"_s).toString())
            + u'\n' + workSources(work).join(u", "_s);
    }

    QList<QStandardItem *> workItems(const QJsonObject &work, const QJsonObject &preferredVariant = {})
    {
        auto *title = new QStandardItem(workTitle(work));
        title->setData(work, Qt::UserRole);
        title->setData(preferredVariant, Qt::UserRole + 1);
        title->setToolTip(workToolTip(work));
        return {title};
    }

    class WorkListDelegate final : public QStyledItemDelegate
    {
    public:
        using QStyledItemDelegate::QStyledItemDelegate;

        QSize sizeHint(const QStyleOptionViewItem &option, const QModelIndex &) const override
        {
            return {0, option.fontMetrics.height() * 3 + 20};
        }

        void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override
        {
            QStyleOptionViewItem panel(option);
            initStyleOption(&panel, index);
            panel.text.clear();
            const QStyle *style = panel.widget ? panel.widget->style() : QApplication::style();
            style->drawControl(QStyle::CE_ItemViewItem, &panel, painter, panel.widget);

            painter->save();
            painter->setClipRect(option.rect);
            painter->setPen(option.palette.color((option.state & QStyle::State_Selected)
                ? QPalette::HighlightedText : QPalette::Text));
            const QRect textRect = option.rect.adjusted(10, 8, -10, -8);
            QFont titleFont = option.font;
            titleFont.setWeight(QFont::DemiBold);
            painter->setFont(titleFont);
            const QFontMetrics titleMetrics(titleFont);
            const QString title = index.data().toString();
            QTextLayout titleLayout(title, titleFont);
            QTextOption textOption;
            textOption.setWrapMode(QTextOption::WrapAtWordBoundaryOrAnywhere);
            titleLayout.setTextOption(textOption);
            titleLayout.beginLayout();
            QTextLine firstLine = titleLayout.createLine();
            if (firstLine.isValid())
                firstLine.setLineWidth(textRect.width());
            titleLayout.endLayout();
            if (firstLine.isValid())
            {
                firstLine.draw(painter, textRect.topLeft());
                const QString remaining = title.mid(firstLine.textLength()).trimmed();
                painter->drawText(textRect.left(), textRect.top() + titleMetrics.height() + titleMetrics.ascent(),
                    titleMetrics.elidedText(remaining, Qt::ElideRight, textRect.width()));
            }

            painter->setFont(option.font);
            const QJsonObject work = index.data(Qt::UserRole).toJsonObject();
            const QStringList sources = workSources(work);
            QString sourceLabel = sources.first(std::min(3, static_cast<int>(sources.size()))).join(u", "_s);
            if (sources.size() > 3)
                sourceLabel += u' ' + Content::LibraryWidget::tr("and %n more", nullptr, sources.size() - 3);

            const int baseline = textRect.bottom() - option.fontMetrics.descent();
            painter->drawText(textRect.left(), baseline,
                option.fontMetrics.elidedText(sourceLabel, Qt::ElideRight, textRect.width()));
            painter->restore();
        }
    };

    BitTorrent::InfoHash variantInfoHash(const QJsonObject &variant)
    {
        QString magnet = variant.value(u"magnet"_s).toString();
        if (magnet.isEmpty() && variant.value(u"url"_s).toString().startsWith(u"magnet:"))
            magnet = variant.value(u"url"_s).toString();
        if (magnet.isEmpty())
        {
            const QString hash = variant.value(u"infoHash"_s).toString();
            if (!hash.isEmpty())
                magnet = u"magnet:?xt=urn:%1%2"_s.arg(hash.size() == 64 ? u"btmh:1220"_s : u"btih:"_s, hash);
        }
        const auto descriptor = BitTorrent::TorrentDescriptor::parse(magnet);
        return descriptor ? descriptor->infoHash() : BitTorrent::InfoHash();
    }

    bool samePublication(const QJsonObject &left, const QJsonObject &right)
    {
        return (left.value(u"sourceId"_s) == right.value(u"sourceId"_s))
            && (left.value(u"url"_s) == right.value(u"url"_s));
    }

    bool sameVariant(const QJsonObject &left, const QJsonObject &right)
    {
        return (left.value(u"sourceId"_s) == right.value(u"sourceId"_s))
            && (left.value(u"id"_s) == right.value(u"id"_s));
    }

    QString variantFacet(const QJsonObject &variant, const QString &key)
    {
        return variant.value(u"facets"_s).toObject().value(key).toString(variant.value(key).toString());
    }

    QString languageName(const QString &language)
    {
        const QLocale locale(language);
        return locale.language() == QLocale::C ? language : locale.nativeLanguageName();
    }

    QString episodeLabel(const QJsonObject &variant)
    {
        const QString episode = variant.value(u"episode"_s).toString();
        const QString season = variant.value(u"season"_s).toString();
        QString label = episode.isEmpty() ? Content::LibraryWidget::tr("Episode not specified")
            : Content::LibraryWidget::tr("Episode %1").arg(episode);
        if (!season.isEmpty())
            label = Content::LibraryWidget::tr("Season %1, %2").arg(season, label);
        return label;
    }

    QJsonObject mergeWork(const QJsonObject &current, const QJsonObject &incoming,
        const QJsonObject &inspectedPublication = {})
    {
        QJsonObject merged = current;
        for (auto it = incoming.constBegin(); it != incoming.constEnd(); ++it)
            merged.insert(it.key(), it.value());

        const QJsonArray incomingVariants = incoming.value(u"variants"_s).toArray();
        const QJsonArray currentVariants = current.value(u"variants"_s).toArray();
        QJsonArray candidates;
        for (const QJsonValue &value : incomingVariants)
        {
            const QJsonObject variant = value.toObject();
            if (inspectedPublication.isEmpty() || samePublication(variant, inspectedPublication))
            {
                candidates.append(value);
            }
            else
            {
                for (const QJsonValue &currentValue : currentVariants)
                {
                    if (sameVariant(currentValue.toObject(), variant))
                    {
                        candidates.append(currentValue);
                        break;
                    }
                }
            }
        }
        for (const QJsonValue &value : currentVariants)
        {
            const QJsonObject variant = value.toObject();
            const bool retain = inspectedPublication.isEmpty()
                ? std::any_of(incomingVariants.cbegin(), incomingVariants.cend(), [&variant](const QJsonValue &candidate)
                {
                    const QJsonObject publication = candidate.toObject();
                    return publication.value(u"needsInspection"_s).toBool() && samePublication(variant, publication);
                })
                : !samePublication(variant, inspectedPublication);
            if (retain)
                candidates.append(value);
        }

        QSet<QString> expandedPublications;
        for (const QJsonValue &value : candidates)
        {
            const QJsonObject variant = value.toObject();
            if (!variant.value(u"needsInspection"_s).toBool())
            {
                expandedPublications.insert(variant.value(u"sourceId"_s).toString()
                    + u'\n' + variant.value(u"url"_s).toString());
            }
        }

        QSet<QString> identities;
        QJsonArray variants;
        for (const QJsonValue &value : candidates)
        {
            const QJsonObject variant = value.toObject();
            const QString publication = variant.value(u"sourceId"_s).toString()
                + u'\n' + variant.value(u"url"_s).toString();
            const QString identity = variant.value(u"sourceId"_s).toString()
                + u'\n' + variant.value(u"id"_s).toString();
            if (identities.contains(identity)
                || (variant.value(u"needsInspection"_s).toBool() && expandedPublications.contains(publication)))
            {
                continue;
            }
            identities.insert(identity);
            variants.append(variant);
        }
        merged.insert(u"variants"_s, variants);
        return merged;
    }
}

class Content::LibraryWidget::WorkFilterModel final : public QSortFilterProxyModel
{
public:
    using QSortFilterProxyModel::QSortFilterProxyModel;

    void setCategory(const QString &category)
    {
        m_category = category;
        invalidateRowsFilter();
    }

protected:
    bool filterAcceptsRow(const int row, const QModelIndex &parent) const override
    {
        const QJsonObject work = sourceModel()->index(row, 0, parent).data(Qt::UserRole).toJsonObject();
        return (m_category == u"all") || (work.value(u"category"_s).toString() == m_category);
    }

private:
    QString m_category = u"all"_s;
};

class Content::LibraryWidget::VariantFilterModel final : public QSortFilterProxyModel
{
public:
    using QSortFilterProxyModel::QSortFilterProxyModel;

    void setShowReported(const bool show)
    {
        if (m_showReported == show)
            return;

        m_showReported = show;
        invalidateRowsFilter();
    }

    void setFacets(const QJsonObject &episode, const QVariant &voice, const QJsonObject &source, const QVariant &language)
    {
        m_episode = episode;
        m_voice = voice;
        m_source = source;
        m_language = language;
        invalidateRowsFilter();
    }

protected:
    bool filterAcceptsRow(const int row, const QModelIndex &parent) const override
    {
        const QJsonObject variant = sourceModel()->index(row, VariantColumn, parent).data(Qt::UserRole).toJsonObject();
        if (!m_showReported && (variant.value(u"transport"_s).toString() == u"video")
            && (variant.value(u"adStatus"_s).toString() == u"reported"))
        {
            return false;
        }

        if (!m_source.isEmpty() && ((variant.value(u"sourceId"_s) != m_source.value(u"sourceId"_s))
            || (m_source.contains(u"player"_s) && (variant.value(u"player"_s).toString() != m_source.value(u"player"_s).toString()))
            || (m_source.contains(u"transport"_s) && (variant.value(u"transport"_s) != m_source.value(u"transport"_s)))))
        {
            return false;
        }

        if (!m_episode.isEmpty() && ((variantFacet(variant, u"episode"_s) != m_episode.value(u"episode"_s).toString())
            || (variantFacet(variant, u"season"_s) != m_episode.value(u"season"_s).toString())))
        {
            return false;
        }
        if (m_voice.isValid() && (variantFacet(variant, u"translation"_s) != m_voice.toString()))
            return false;
        if (m_language.isValid())
        {
            const QJsonArray languages = variant.value(u"languages"_s).toArray();
            if (m_language.toString().isEmpty() ? !languages.isEmpty() : !languages.contains(m_language.toString()))
                return false;
        }
        return QSortFilterProxyModel::filterAcceptsRow(row, parent);
    }

private:
    bool m_showReported = false;
    QJsonObject m_episode;
    QVariant m_voice;
    QJsonObject m_source;
    QVariant m_language;
};

Content::LibraryWidget::LibraryWidget(IGUIApplication *app, QWidget *parent)
    : GUIApplicationComponent(app, parent)
{
    setObjectName(u"contentLibrary"_s);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(16, 12, 16, 12);
    layout->setSpacing(10);

    m_navigation = new QWidget(this);
    auto *navigationLayout = new QVBoxLayout(m_navigation);
    navigationLayout->setContentsMargins(0, 0, 0, 0);
    navigationLayout->setSpacing(10);
    auto *searchBar = new QHBoxLayout;
    m_query = new QLineEdit(this);
    m_query->setObjectName(u"contentQuery"_s);
    m_query->setPlaceholderText(tr("Search games, films, anime, books…"));
    m_query->setClearButtonEnabled(true);
    searchBar->addWidget(m_query, 1);
    m_category = new QComboBox(this);
    m_category->setObjectName(u"contentCategory"_s);
    for (const QString &category : {u"all"_s, u"games"_s, u"movies"_s, u"tv"_s,
        u"anime"_s, u"software"_s, u"books"_s, u"manga"_s})
    {
        m_category->addItem(categoryName(category), category);
    }
    searchBar->addWidget(m_category);
    m_search = new QPushButton(UIThemeManager::instance()->getIcon(u"edit-find"_s), tr("Search"), this);
    m_search->setObjectName(u"contentSearch"_s);
    searchBar->addWidget(m_search);
    auto *sources = new QPushButton(tr("Sources"), this);
    sources->setObjectName(u"contentSources"_s);
    searchBar->addWidget(sources);
    navigationLayout->addLayout(searchBar);
    m_searchProgress = new QProgressBar(this);
    m_searchProgress->setObjectName(u"contentSearchProgress"_s);
    m_searchProgress->setRange(0, 0);
    m_searchProgress->setTextVisible(false);
    m_searchProgress->setMaximumHeight(4);
    m_searchProgress->hide();
    navigationLayout->addWidget(m_searchProgress);

    m_views = new QTabBar(this);
    m_views->setExpanding(false);
    m_views->addTab(tr("Search"));
    m_views->addTab(tr("Favorites"));
    m_views->addTab(tr("History"));
    m_views->addTab(tr("Downloads"));
    navigationLayout->addWidget(m_views);
    layout->addWidget(m_navigation);
    m_pages = new QStackedWidget(this);
    connect(m_pages, &QStackedWidget::currentChanged, this, [this]
    {
        const bool reading = isReading();
        const bool prerequisite = m_browserPrerequisite && (m_pages->currentWidget() == m_browserPrerequisite);
        m_navigation->setVisible(!reading && !prerequisite);
        emit readerActiveChanged(reading);
    });
    layout->addWidget(m_pages, 1);
    m_splitter = new QSplitter(this);
    m_splitter->setChildrenCollapsible(false);
    m_pages->addWidget(m_splitter);

    auto *results = new QWidget(m_splitter);
    results->setMinimumWidth(220);
    auto *resultLayout = new QVBoxLayout(results);
    resultLayout->setContentsMargins(0, 0, 0, 0);
    m_searchModel = new QStandardItemModel(this);
    m_searchModel->setSortRole(Qt::UserRole + 2);
    m_searchProxy = new WorkFilterModel(this);
    m_searchProxy->setSourceModel(m_searchModel);
    m_libraryModel = new QStandardItemModel(this);
    m_works = new QTableView(results);
    m_works->setObjectName(u"contentResults"_s);
    m_works->setModel(m_searchProxy);
    configureTable(m_works);
    m_works->setItemDelegate(new WorkListDelegate(m_works));
    m_works->verticalHeader()->setSectionResizeMode(QHeaderView::ResizeToContents);
    m_works->horizontalHeader()->hide();
    m_works->horizontalHeader()->setSectionResizeMode(QHeaderView::Stretch);
    resultLayout->addWidget(m_works, 1);
    m_emptyResults = new QLabel(results);
    m_emptyResults->setObjectName(u"contentEmptyResults"_s);
    m_emptyResults->setAlignment(Qt::AlignCenter);
    m_emptyResults->setWordWrap(true);
    m_emptyResults->hide();
    resultLayout->addWidget(m_emptyResults, 1);
    auto *statusRow = new QHBoxLayout;
    m_status = new QLabel(results);
    m_status->setWordWrap(true);
    statusRow->addWidget(m_status, 1);
    m_openSource = new QPushButton(tr("Open source"), results);
    m_openSource->hide();
    statusRow->addWidget(m_openSource);
    m_retry = new QPushButton(tr("Retry"), results);
    m_retry->hide();
    statusRow->addWidget(m_retry);
    m_expand = new QPushButton(tr("Search more"), results);
    m_expand->hide();
    statusRow->addWidget(m_expand);
    resultLayout->addLayout(statusRow);
    m_splitter->addWidget(results);

    m_details = new QWidget(m_splitter);
    m_details->setMinimumWidth(320);
    auto *panelLayout = new QVBoxLayout(m_details);
    panelLayout->setContentsMargins(12, 0, 0, 0);
    panelLayout->setSpacing(10);
    m_title = new WorkTitleLabel(m_details);
    m_title->setObjectName(u"contentTitle"_s);
    m_title->setWordWrap(true);
    m_title->setTextFormat(Qt::PlainText);
    m_title->setTextInteractionFlags(Qt::TextSelectableByMouse);
    QFont titleFont = m_title->font();
    titleFont.setPointSize(titleFont.pointSize() + 4);
    titleFont.setBold(true);
    m_title->setFont(titleFont);
    auto *heading = new QHBoxLayout;
    heading->addWidget(m_title);
    m_favorite = new QToolButton(m_details);
    m_favorite->setObjectName(u"contentSave"_s);
    m_favorite->setCheckable(true);
    m_favorite->setAutoRaise(true);
    m_favorite->setToolButtonStyle(Qt::ToolButtonIconOnly);
    heading->addWidget(m_favorite, 0, Qt::AlignTop);
    heading->addStretch();
    panelLayout->addLayout(heading);
    m_detailPages = new QTabWidget(m_details);
    m_detailPages->setObjectName(u"contentDetailPages"_s);
    auto *versions = new QWidget(m_detailPages);
    auto *versionLayout = new QVBoxLayout(versions);
    versionLayout->setContentsMargins(0, 10, 0, 0);
    versionLayout->setSpacing(10);
    m_variantFacets = new QWidget(versions);
    auto *facets = new FlowLayout(m_variantFacets, 0);
    m_episode = new QComboBox(m_variantFacets);
    m_episode->setObjectName(u"contentEpisode"_s);
    m_episode->addItem(tr("All episodes"));
    m_voice = new QComboBox(m_variantFacets);
    m_voice->setObjectName(u"contentVoice"_s);
    m_voice->addItem(tr("All voices"));
    m_source = new QComboBox(m_variantFacets);
    m_source->setObjectName(u"contentVariantSource"_s);
    m_source->addItem(tr("All sources"));
    m_language = new QComboBox(m_variantFacets);
    m_language->setObjectName(u"contentLanguage"_s);
    m_language->addItem(tr("All languages"));
    for (QComboBox *combo : {m_episode, m_voice, m_source, m_language})
    {
        combo->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
        combo->setMinimumContentsLength(combo->itemText(0).size());
        facets->addWidget(combo);
        connect(combo, &QComboBox::currentIndexChanged, this, [this, combo]
        {
            combo->setMinimumContentsLength(std::min(qsizetype(24), std::max(combo->itemText(0).size(), combo->currentText().size())));
            combo->setToolTip(combo->currentText());
            filterVariants();
        });
    }
    versionLayout->addWidget(m_variantFacets);
    m_variantFilter = new QLineEdit(versions);
    m_variantFilter->setObjectName(u"contentVariantFilter"_s);
    m_variantFilter->setPlaceholderText(tr("Find a version…"));
    m_variantFilter->setClearButtonEnabled(true);
    versionLayout->addWidget(m_variantFilter);
    m_showReported = new QCheckBox(tr("Include reported ads"), versions);
    m_showReported->setObjectName(u"contentIncludeReportedAds"_s);
    m_showReported->hide();
    versionLayout->addWidget(m_showReported);
    m_variantModel = new QStandardItemModel(this);
    m_variantModel->setHorizontalHeaderLabels({tr("Variant"), tr("Source"), tr("Size"), tr("Seeds")});
    m_variantProxy = new VariantFilterModel(this);
    m_variantProxy->setSourceModel(m_variantModel);
    m_variantProxy->setFilterRole(Qt::UserRole + 1);
    m_variants = new QTableView(versions);
    m_variants->setObjectName(u"contentVariants"_s);
    m_variants->setModel(m_variantProxy);
    configureTable(m_variants);
    m_variants->horizontalHeader()->setSectionResizeMode(VariantColumn, QHeaderView::Stretch);
    m_variants->horizontalHeader()->setSectionResizeMode(SourceColumn, QHeaderView::Interactive);
    m_variants->setColumnWidth(SourceColumn, 100);
    m_variants->horizontalHeader()->setSectionResizeMode(SizeColumn, QHeaderView::ResizeToContents);
    m_variants->horizontalHeader()->setSectionResizeMode(SeedsColumn, QHeaderView::ResizeToContents);
    m_detailsProgress = new QProgressBar(versions);
    m_detailsProgress->setObjectName(u"contentDetailsProgress"_s);
    m_detailsProgress->setRange(0, 0);
    m_detailsProgress->setTextVisible(false);
    m_detailsProgress->setMaximumHeight(4);
    m_detailsProgress->hide();
    versionLayout->addWidget(m_detailsProgress);
    versionLayout->addWidget(m_variants, 1);
    m_emptyVariants = new QLabel(versions);
    m_emptyVariants->setObjectName(u"contentEmptyVariants"_s);
    m_emptyVariants->setAlignment(Qt::AlignCenter);
    m_emptyVariants->setWordWrap(true);
    m_emptyVariants->hide();
    versionLayout->addWidget(m_emptyVariants, 1);
    m_detailPages->addTab(versions, tr("Versions"));
    auto *actions = new FlowLayout(0);
    m_open = new QPushButton(tr("Open"), m_details);
    m_open->setObjectName(u"contentOpen"_s);
    m_open->setDefault(true);
    actions->addWidget(m_open);
    m_download = new QPushButton(tr("Download"), m_details);
    m_download->hide();
    actions->addWidget(m_download);
    m_share = new QToolButton(m_details);
    m_share->setObjectName(u"contentShare"_s);
    m_share->setText(tr("Share"));
    m_share->setToolButtonStyle(Qt::ToolButtonTextOnly);
    actions->addWidget(m_share);
    m_remove = new QPushButton(tr("Remove"), m_details);
    m_remove->hide();
    actions->addWidget(m_remove);
    panelLayout->addLayout(actions);
    m_notes = new QTextBrowser(m_detailPages);
    m_notes->setObjectName(u"contentNotes"_s);
    m_notes->setFrameShape(QFrame::NoFrame);
    m_notes->setOpenExternalLinks(true);
    m_notes->document()->setDocumentMargin(0);
    m_detailPages->addTab(m_notes, tr("Details"));
    panelLayout->addWidget(m_detailPages, 1);
    m_details->hide();
    m_splitter->addWidget(m_details);
    m_splitter->setSizes({320, 640});

    m_downloads = new DownloadsWidget(m_pages);
    m_pages->addWidget(m_downloads);
    connect(m_downloads, &DownloadsWidget::statusChanged, m_status, &QLabel::setText);
    connect(m_downloads, &DownloadsWidget::preparationChanged, this, &LibraryWidget::updateActions);

    connect(m_query, &QLineEdit::returnPressed, this, [this] { search(); });
    connect(m_search, &QPushButton::clicked, this, [this]
    {
        if (m_searchRequest)
        {
            Client::instance()->cancel(m_searchRequest);
            m_searchRequest = 0;
            m_search->setText(tr("Search"));
            updateSearchView();
        }
        else
        {
            search();
        }
    });
    connect(m_expand, &QPushButton::clicked, this, [this] { search(true); });
    connect(m_category, &QComboBox::currentIndexChanged, this, [this]
    {
        m_searchProxy->setCategory(m_category->currentData().toString());
        updateSearchView();
    });
    connect(m_openSource, &QPushButton::clicked, this, [this] { QDesktopServices::openUrl(m_failureUrl); });
    connect(m_retry, &QPushButton::clicked, this, &LibraryWidget::startResolve);
    connect(sources, &QPushButton::clicked, this, [this] { showSources(); });
    connect(m_views, &QTabBar::currentChanged, this, &LibraryWidget::changeView);
    connect(m_works->selectionModel(), &QItemSelectionModel::currentRowChanged, this, &LibraryWidget::selectWork);
    connect(m_variants->selectionModel(), &QItemSelectionModel::currentRowChanged, this, &LibraryWidget::selectVariant);
    connect(m_variantFilter, &QLineEdit::textChanged, this, &LibraryWidget::filterVariants);
    connect(m_showReported, &QCheckBox::toggled, this, &LibraryWidget::filterVariants);
    connect(m_variants, &QTableView::doubleClicked, this, [this] { resolveVariant(); });
    m_variants->setContextMenuPolicy(Qt::CustomContextMenu);
    connect(m_variants, &QTableView::customContextMenuRequested, this, [this](const QPoint &position)
    {
        const QModelIndex index = m_variants->indexAt(position);
        if (!index.isValid())
            return;

        m_variants->selectRow(index.row());
        const QUrl url(currentVariant().value(u"url"_s).toString());
        if (url.host().isEmpty() || ((url.scheme() != u"http") && (url.scheme() != u"https")))
            return;

        QMenu menu(this);
        menu.addAction(tr("Open source"), this, [url] { QDesktopServices::openUrl(url); });
        menu.exec(m_variants->viewport()->mapToGlobal(position));
    });
    connect(m_open, &QPushButton::clicked, this, [this]
    {
        if (!m_torrentPlayback.isEmpty())
        {
            clearTorrentWatch();
            m_status->setText(tr("Watch cancelled"));
            updateActions();
        }
        else
        {
            resolveVariant();
        }
    });
    connect(m_download, &QPushButton::clicked, this, &LibraryWidget::downloadVariant);
    connect(m_favorite, &QToolButton::toggled, this, &LibraryWidget::updateActions);
    connect(m_favorite, &QToolButton::clicked, this, &LibraryWidget::toggleFavorite);
    connect(m_share, &QToolButton::clicked, this, &LibraryWidget::shareWork);
    connect(Media::Player::instance(), &Media::Player::subtitleSettingsRequested, this, [this] { showSources(u"ai"_s); });
    connect(m_remove, &QPushButton::clicked, this, [this]
    {
        m_removeRequest = Client::instance()->request(u"remove"_s, {{u"id"_s, m_work.value(u"id"_s)}});
    });
    connect(Client::instance(), &Client::eventReceived, this, &LibraryWidget::handleEvent);
    connect(Media::Player::instance(), &Media::Player::playbackStarted, this, [this](const QString &identity)
    {
        if (m_playback.isEmpty() || (identity != m_playback.value(u"identity"_s).toString()))
            return;

        const QJsonObject work = m_playback.value(u"work"_s).toObject();
        m_rememberRequests.insert(Client::instance()->request(u"remember"_s,
            {{u"work"_s, work}, {u"variant"_s, m_playback.value(u"variant"_s)},
                {u"variantFilters"_s, m_playback.value(u"variantFilters"_s)}, {u"opened"_s, true}}));
        m_status->setText(tr("Opened %1").arg(work.value(u"title"_s).toString()));
    });
    connect(app->addTorrentManager(), &AddTorrentManager::torrentAdded, this,
        [this](const QString &, BitTorrent::Torrent *torrent)
    {
        if (!m_torrentPlayback.isEmpty() && (BitTorrent::Session::instance()->findTorrent(m_watchInfoHash) == torrent))
            watchTorrent(torrent);
    });
    connect(app->addTorrentManager(), &GUIAddTorrentManager::addTorrentCancelled, this, [this](const BitTorrent::InfoHash &infoHash)
    {
        if (!m_torrentPlayback.isEmpty() && (infoHash == m_watchInfoHash))
        {
            clearTorrentWatch();
            m_status->setText(tr("Watch cancelled"));
            updateActions();
        }
    });
    connect(app->addTorrentManager(), &AddTorrentManager::addTorrentFailed, this,
        [this](const QString &source, const BitTorrent::AddTorrentError &reason)
    {
        if (!m_torrentPlayback.isEmpty() && (source == m_torrentPlayback.value(u"source"_s).toString()))
        {
            clearTorrentWatch();
            m_status->setText(reason.message);
            updateActions();
        }
    });
    connect(BitTorrent::Session::instance(), &BitTorrent::Session::torrentMetadataReceived, this,
        [this](BitTorrent::Torrent *torrent)
    {
        if ((m_watchTorrent == torrent) && !m_torrentPlayback.isEmpty())
            watchTorrent(torrent);
    });
    connect(BitTorrent::Session::instance(), &BitTorrent::Session::torrentFinishedChecking, this,
        [this](BitTorrent::Torrent *torrent)
    {
        if ((m_watchTorrent == torrent) && !m_torrentPlayback.isEmpty())
            watchTorrent(torrent);
    });
    connect(BitTorrent::Session::instance(), &BitTorrent::Session::torrentStopped, this,
        [this](BitTorrent::Torrent *torrent)
    {
        if (m_watchTorrent == torrent)
        {
            clearTorrentWatch();
            m_status->setText(tr("Watch cancelled"));
            updateActions();
        }
    });
    connect(BitTorrent::Session::instance(), &BitTorrent::Session::torrentAboutToBeRemoved, this,
        [this](BitTorrent::Torrent *torrent)
    {
        if (m_watchTorrent == torrent)
        {
            clearTorrentWatch();
            updateActions();
        }
    });
    loadLibrary();
}

Content::LibraryWidget::~LibraryWidget()
{
    clearTorrentWatch();
    Client::instance()->cancel(m_searchRequest);
    Client::instance()->cancel(m_detailsRequest);
    Client::instance()->cancel(m_resolveRequest);
}

void Content::LibraryWidget::giveFocusToSearchInput()
{
    m_query->setFocus(Qt::ShortcutFocusReason);
    m_query->selectAll();
}

void Content::LibraryWidget::acceptSharedLink(const QUrl &url)
{
    Client::instance()->cancel(m_searchRequest);
    m_searchRequest = 0;
    m_search->setText(tr("Search"));
    m_searchModel->removeRows(0, m_searchModel->rowCount());
    m_workRows.clear();
    m_views->setCurrentIndex(0);
    m_pages->setCurrentIndex(0);
    clearDetails();
    m_linkRequest = Client::instance()->request(u"openLink"_s, {{u"url"_s, url.toString(QUrl::FullyEncoded)}});
    m_status->setText(tr("Opening shared title…"));
    updateSearchView();
}

void Content::LibraryWidget::downloadMedia(const QJsonObject &params)
{
    QJsonObject request = params;
    const QString identity = params.value(u"options"_s).toObject().value(u"identity"_s).toString();
    if (!identity.isEmpty() && (identity == m_playback.value(u"identity"_s).toString()))
    {
        request.insert(u"work"_s, m_playback.value(u"work"_s));
        request.insert(u"variant"_s, m_playback.value(u"variant"_s));
    }
    m_downloads->enqueue(request);
}

bool Content::LibraryWidget::openLocalFile()
{
    ReaderWidget *reader = ensureReader();
    if (!reader->chooseFile())
        return false;

    m_pages->setCurrentWidget(reader);
    return true;
}

bool Content::LibraryWidget::isReading() const
{
    return m_reader && (m_pages->currentWidget() == m_reader);
}

void Content::LibraryWidget::search(const bool expanded)
{
    const QString query = m_query->text().trimmed();
    if (query.isEmpty())
    {
        giveFocusToSearchInput();
        return;
    }
    Client::instance()->cancel(m_searchRequest);
    clearDetails();
    m_searchModel->removeRows(0, m_searchModel->rowCount());
    m_workRows.clear();
    m_views->setCurrentIndex(0);
    m_pages->setCurrentIndex(0);
    m_expand->hide();
    m_expandedSearch = expanded;
    m_status->setToolTip({});
    m_searchRequest = Client::instance()->request(u"search"_s, {
        {u"query"_s, query}, {u"category"_s, u"all"_s}, {u"expanded"_s, expanded},
        {u"language"_s, QLocale(Preferences::instance()->getLocale()).bcp47Name()}
    });
    m_search->setText(tr("Stop"));
    m_status->clear();
    updateSearchView();
}

void Content::LibraryWidget::changeView()
{
    clearDetails();
    m_pages->setCurrentIndex(m_views->currentIndex() == 3 ? 1 : 0);
    if (m_views->currentIndex() == 3)
    {
        m_downloads->refresh();
        return;
    }
    QAbstractItemModel *model = (m_views->currentIndex() == 0) ? static_cast<QAbstractItemModel *>(m_searchProxy) : m_libraryModel;
    if (m_works->model() != model)
    {
        QItemSelectionModel *previousSelection = m_works->selectionModel();
        m_works->setModel(model);
        delete previousSelection;
        connect(m_works->selectionModel(), &QItemSelectionModel::currentRowChanged, this, &LibraryWidget::selectWork);
    }
    m_remove->setVisible(m_views->currentIndex() > 0);
    m_expand->hide();
    if (m_views->currentIndex() > 0)
        loadLibrary();
    else
    {
        m_status->clear();
        if (m_works->currentIndex().isValid())
            selectWork();
    }
    updateSearchView();
}

void Content::LibraryWidget::updateSearchView()
{
    const bool searching = m_views->currentIndex() == 0;
    m_searchProgress->setVisible(searching && m_searchRequest);
    const bool empty = searching && (m_searchProxy->rowCount() == 0);
    m_emptyResults->setVisible(empty);
    m_works->setVisible(!empty);
    m_emptyResults->setText(m_searchModel->rowCount() > 0 ? tr("No titles in this category")
        : m_searchRequest ? tr("No results yet") : tr("No results"));
    if (searching && !m_works->currentIndex().isValid() && !m_work.isEmpty())
        clearDetails();
    if (searching && !m_works->currentIndex().isValid() && !empty)
        m_works->selectRow(0);
    if (searching && m_works->currentIndex().isValid()
        && (m_works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject().value(u"id"_s)
            != m_work.value(u"id"_s)))
    {
        selectWork();
    }
}

void Content::LibraryWidget::loadLibrary()
{
    m_libraryModel->removeRows(0, m_libraryModel->rowCount());
    m_libraryEntries.clear();
    m_libraryRequest = Client::instance()->request(u"library"_s);
    if (m_views->currentIndex() > 0)
        m_status->setText(tr("Loading library…"));
}

void Content::LibraryWidget::updateLibraryEntries(const QJsonArray &entries)
{
    const bool libraryView = (m_views->currentIndex() == 1) || (m_views->currentIndex() == 2);
    for (const QJsonValue &value : entries)
    {
        const QJsonObject entry = value.toObject();
        const QJsonObject work = entry.value(u"work"_s).toObject();
        const QString workId = work.value(u"id"_s).toString();
        m_libraryEntries.insert(workId, entry);
        if (!libraryView)
            continue;

        int row = -1;
        for (int index = 0; index < m_libraryModel->rowCount(); ++index)
        {
            if (m_libraryModel->item(index, 0)->data(Qt::UserRole).toJsonObject().value(u"id"_s).toString() == workId)
            {
                row = index;
                break;
            }
        }
        const bool visible = (m_views->currentIndex() == 1) ? entry.value(u"favorite"_s).toBool()
            : !entry.value(u"lastOpened"_s).toString().isEmpty();
        if (!visible)
        {
            if (row >= 0)
                m_libraryModel->removeRow(row);
        }
        else
        {
            const QJsonObject variant = entry.value(u"variant"_s).toObject();
            const QJsonObject preferred {{u"id"_s, variant.value(u"id"_s)}, {u"sourceId"_s, variant.value(u"sourceId"_s)}};
            if (row < 0)
                m_libraryModel->appendRow(workItems(work, preferred));
            else
            {
                QStandardItem *item = m_libraryModel->item(row, 0);
                item->setData(work, Qt::UserRole);
                item->setData(preferred, Qt::UserRole + 1);
                item->setText(workTitle(work));
                item->setToolTip(workToolTip(work));
            }
        }
    }
    if (libraryView)
    {
        m_status->setText(tr("%n title(s)", nullptr, m_libraryModel->rowCount()));
        if ((m_libraryModel->rowCount() > 0) && !m_works->currentIndex().isValid())
            m_works->selectRow(0);
        else if (m_libraryModel->rowCount() == 0)
            clearDetails();
    }
}

QJsonObject Content::LibraryWidget::libraryEntry(const QJsonObject &work) const
{
    const QString id = work.value(u"id"_s).toString();
    if (m_libraryEntries.contains(id))
        return m_libraryEntries.value(id);

    for (auto it = m_libraryEntries.cbegin(); it != m_libraryEntries.cend(); ++it)
    {
        const QJsonArray savedVariants = it.value().value(u"work"_s).toObject().value(u"variants"_s).toArray();
        for (const QJsonValue &value : work.value(u"variants"_s).toArray())
        {
            const QJsonObject variant = value.toObject();
            const BitTorrent::InfoHash hash = variantInfoHash(variant);
            for (const QJsonValue &savedValue : savedVariants)
            {
                const QJsonObject saved = savedValue.toObject();
                const bool samePublication = !variant.value(u"id"_s).toString().isEmpty()
                    && (variant.value(u"id"_s) == saved.value(u"id"_s))
                    && (variant.value(u"sourceId"_s) == saved.value(u"sourceId"_s));
                if (samePublication || (hash.isValid() && (hash == variantInfoHash(saved))))
                    return it.value();
            }
        }
    }
    return {};
}

void Content::LibraryWidget::clearDetails()
{
    Client::instance()->cancel(m_detailsRequest);
    Client::instance()->cancel(m_linkRequest);
    m_detailsRequest = 0;
    m_inspectedPublications.clear();
    m_linkRequest = 0;
    m_work = {};
    m_variantModel->removeRows(0, m_variantModel->rowCount());
    for (QComboBox *combo : {m_episode, m_voice, m_source, m_language})
    {
        const QSignalBlocker blocker(combo);
        combo->setCurrentIndex(0);
    }
    m_variantFacets->hide();
    m_variantFilter->clear();
    m_showReported->setChecked(false);
    applyVariantFilters();
    m_detailPages->setCurrentIndex(0);
    m_details->hide();
    m_openSource->hide();
    m_retry->hide();
    m_failureUrl.clear();
}

void Content::LibraryWidget::upsertWork(QJsonObject work)
{
    const QString id = work.value(u"id"_s).toString();
    if (id.isEmpty())
        return;

    const double relevance = work.take(u"relevance"_s).toDouble();
    const QString selectedId = (m_views->currentIndex() == 0)
        ? m_works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject().value(u"id"_s).toString() : QString();
    const QSignalBlocker selectionBlocker(m_works->selectionModel());
    const int row = m_workRows.value(id, -1);
    bool reorder = false;
    if (row < 0)
    {
        const int lastRow = m_searchModel->rowCount() - 1;
        reorder = (lastRow >= 0) && (relevance > m_searchModel->item(lastRow, 0)->data(Qt::UserRole + 2).toDouble());
        m_workRows.insert(id, m_searchModel->rowCount());
        const QList<QStandardItem *> items = workItems(work);
        items.first()->setData(relevance, Qt::UserRole + 2);
        m_searchModel->appendRow(items);
    }
    else
    {
        reorder = relevance != m_searchModel->item(row, 0)->data(Qt::UserRole + 2).toDouble();
        const QJsonObject merged = mergeWork(m_searchModel->item(row, 0)->data(Qt::UserRole).toJsonObject(), work);
        m_searchModel->item(row, 0)->setData(merged, Qt::UserRole);
        m_searchModel->item(row, 0)->setData(relevance, Qt::UserRole + 2);
        m_searchModel->item(row, 0)->setText(workTitle(merged));
        m_searchModel->item(row, 0)->setToolTip(workToolTip(merged));
    }

    if (reorder)
    {
        m_searchModel->sort(0, Qt::DescendingOrder);
        m_workRows.clear();
        for (int index = 0; index < m_searchModel->rowCount(); ++index)
        {
            const QString workId = m_searchModel->item(index, 0)->data(Qt::UserRole).toJsonObject().value(u"id"_s).toString();
            m_workRows.insert(workId, index);
        }
        if (!selectedId.isEmpty())
            m_works->selectRow(m_searchProxy->mapFromSource(m_searchModel->index(m_workRows.value(selectedId), 0)).row());
    }

    if (selectedId == id)
    {
        showWork(m_searchModel->item(m_workRows.value(id), 0)->data(Qt::UserRole).toJsonObject());
        inspectWork();
    }
}

void Content::LibraryWidget::selectWork()
{
    clearDetails();
    const QModelIndex index = m_works->currentIndex();
    if (!index.isValid())
        return;

    const QModelIndex title = index.siblingAtColumn(0);
    const QJsonObject work = title.data(Qt::UserRole).toJsonObject();
    const QJsonValue filters = (m_views->currentIndex() == 0) ? QJsonValue()
        : m_libraryEntries.value(work.value(u"id"_s).toString()).value(u"variantFilters"_s);
    showWork(work, title.data(Qt::UserRole + 1).toJsonObject(), true, filters);
    inspectWork();
}

QJsonObject Content::LibraryWidget::variantFilters() const
{
    QJsonObject filters;
    if (m_episode->currentIndex() > 0)
        filters.insert(u"episode"_s, m_episode->currentData().toJsonObject());
    if (m_voice->currentIndex() > 0)
        filters.insert(u"translation"_s, m_voice->currentData().toString());
    if (m_source->currentIndex() > 0)
        filters.insert(u"source"_s, m_source->currentData().toJsonObject());
    if (m_language->currentIndex() > 0)
        filters.insert(u"language"_s, m_language->currentData().toString());
    return filters;
}

void Content::LibraryWidget::updateVariantFacets(const QJsonArray &variants, const QJsonObject &preferredVariant,
    const bool revealPreferred, const QJsonValue &filters)
{
    QList<QPair<QString, QVariant>> episodes;
    QList<QPair<QString, QVariant>> voices;
    QList<QPair<QString, QVariant>> sources;
    QList<QPair<QString, QVariant>> languages;
    QSet<QPair<QString, QString>> episodeKeys;
    QSet<QString> voiceKeys;
    QSet<QString> languageKeys;
    QList<QJsonObject> sourceKeys;
    QHash<QPair<QString, QString>, QSet<QString>> transports;
    bool episodic = false;
    bool hasLanguages = false;
    const QString category = m_work.value(u"category"_s).toString();
    const bool reading = (category == u"books") || (category == u"manga");
    m_voice->setItemText(0, reading ? tr("All translations") : tr("All voices"));
    QJsonObject preferred;
    for (const QJsonValue &value : variants)
    {
        const QJsonObject variant = value.toObject();
        transports[{variant.value(u"sourceId"_s).toString(), variant.value(u"player"_s).toString()}]
            .insert(variant.value(u"transport"_s).toString());
        if (sameVariant(variant, preferredVariant))
            preferred = variant;
    }
    for (const QJsonValue &value : variants)
    {
        const QJsonObject variant = value.toObject();
        if (variant.value(u"needsInspection"_s).toBool())
            continue;
        const QString sourceId = variant.value(u"sourceId"_s).toString();
        const QString player = variant.value(u"player"_s).toString();
        const QString transport = variant.value(u"transport"_s).toString();
        const QJsonObject source {{u"sourceId"_s, sourceId}, {u"player"_s, player}, {u"transport"_s, transport}};
        if (!sourceKeys.contains(source))
        {
            sourceKeys.append(source);
            QString label = variant.value(u"sourceName"_s).toString(sourceId);
            if (!player.isEmpty())
                label += u" [%1]"_s.arg(player);
            if (transports.value({sourceId, player}).size() > 1)
                label += u" [%1]"_s.arg(transport == u"torrent" ? tr("Torrent") : transport == u"video" ? tr("Video") : transport);
            sources.append({label, source});
        }
        const QString episode = variantFacet(variant, u"episode"_s);
        const QString season = variantFacet(variant, u"season"_s);
        episodic |= !episode.isEmpty() || !season.isEmpty();
        if (!episodeKeys.contains({season, episode}))
        {
            episodeKeys.insert({season, episode});
            const QJsonObject episodeFacet {{u"episode"_s, episode}, {u"season"_s, season}};
            episodes.append({episodeLabel(episodeFacet), episodeFacet});
        }
        const QString voice = variantFacet(variant, u"translation"_s);
        if (!voiceKeys.contains(voice))
        {
            voiceKeys.insert(voice);
            voices.append({voice.isEmpty() ? (reading ? tr("Translation not specified") : tr("Voice not specified")) : voice, voice});
        }
        QJsonArray declaredLanguages = variant.value(u"languages"_s).toArray();
        hasLanguages |= !declaredLanguages.isEmpty();
        if (declaredLanguages.isEmpty())
            declaredLanguages.append(QString());
        for (const QJsonValue &value : declaredLanguages)
        {
            const QString language = value.toString();
            if (!languageKeys.contains(language))
            {
                languageKeys.insert(language);
                languages.append({language.isEmpty() ? tr("Language not specified") : languageName(language), language});
            }
        }
    }

    QCollator collator;
    collator.setNumericMode(true);
    std::stable_sort(episodes.begin(), episodes.end(), [&collator](const auto &left, const auto &right)
    {
        const QJsonObject first = left.second.toJsonObject();
        const QJsonObject second = right.second.toJsonObject();
        const QString firstSeason = first.value(u"season"_s).toString();
        const QString secondSeason = second.value(u"season"_s).toString();
        if (firstSeason != secondSeason)
            return collator.compare(firstSeason, secondSeason) < 0;
        const QString firstEpisode = first.value(u"episode"_s).toString();
        const QString secondEpisode = second.value(u"episode"_s).toString();
        if (firstEpisode.isEmpty() != secondEpisode.isEmpty())
            return !firstEpisode.isEmpty();
        return collator.compare(firstEpisode, secondEpisode) < 0;
    });

    const auto populate = [&filters, reading](QComboBox *combo, const QString &key,
        const QList<QPair<QString, QVariant>> &items, const QVariant &preferred)
    {
        const QSignalBlocker blocker(combo);
        const QJsonValue saved = filters.toObject().value(key);
        const QVariant selected = filters.isObject()
            ? (saved.isObject() ? QVariant(saved.toObject()) : saved.isString() ? QVariant(saved.toString()) : QVariant())
            : preferred.isValid() ? preferred : combo->currentData();
        QString selectedText = combo->currentText();
        if (filters.isObject())
        {
            if (key == u"episode")
                selectedText = episodeLabel(saved.toObject());
            else if (key == u"source")
            {
                const QJsonObject source = saved.toObject();
                selectedText = source.value(u"sourceId"_s).toString();
                const QString player = source.value(u"player"_s).toString();
                if (!player.isEmpty())
                    selectedText += u" [%1]"_s.arg(player);
            }
            else if (key == u"language")
                selectedText = saved.toString().isEmpty() ? tr("Language not specified") : languageName(saved.toString());
            else
                selectedText = saved.toString().isEmpty()
                    ? (reading ? tr("Translation not specified") : tr("Voice not specified")) : saved.toString();
        }
        while (combo->count() > 1)
            combo->removeItem(1);
        for (const auto &[label, data] : items)
            combo->addItem(label, data);
        int index = combo->findData(selected);
        if ((index < 0) && selected.isValid())
        {
            combo->addItem(selectedText, selected);
            index = combo->count() - 1;
        }
        combo->setCurrentIndex(index < 0 ? 0 : index);
        combo->setMinimumContentsLength(std::min(qsizetype(24), std::max(combo->itemText(0).size(), combo->currentText().size())));
        combo->setToolTip(combo->currentText());
    };
    if (revealPreferred && !episodic && m_variantFacets->isHidden())
        preferred = {};
    const QJsonObject previousEpisode {{u"episode"_s, variantFacet(preferredVariant, u"episode"_s)},
        {u"season"_s, variantFacet(preferredVariant, u"season"_s)}};
    const bool followEpisode = revealPreferred || ((m_episode->currentIndex() > 0)
        && (m_episode->currentData().toJsonObject() == previousEpisode));
    const bool followVoice = revealPreferred || ((m_voice->currentIndex() > 0)
        && (m_voice->currentData().toString() == variantFacet(preferredVariant, u"translation"_s)));
    populate(m_episode, u"episode"_s, episodes, (preferred.isEmpty() || !followEpisode) ? QVariant() : QVariant(QJsonObject {
        {u"episode"_s, variantFacet(preferred, u"episode"_s)}, {u"season"_s, variantFacet(preferred, u"season"_s)}}));
    populate(m_voice, u"translation"_s, voices, (preferred.isEmpty() || !followVoice) ? QVariant() : QVariant(variantFacet(preferred, u"translation"_s)));
    populate(m_source, u"source"_s, sources, (preferred.isEmpty() || !revealPreferred) ? QVariant() : QVariant(QJsonObject {
        {u"sourceId"_s, preferred.value(u"sourceId"_s)}, {u"player"_s, preferred.value(u"player"_s).toString()},
        {u"transport"_s, preferred.value(u"transport"_s)}}));
    const QJsonArray preferredLanguages = preferred.value(u"languages"_s).toArray();
    populate(m_language, u"language"_s, languages, (revealPreferred && (preferredLanguages.size() == 1))
        ? QVariant(preferredLanguages.first().toString()) : QVariant());
    m_variantFacets->setVisible(episodic || hasLanguages || (m_episode->currentIndex() > 0)
        || (m_voice->currentIndex() > 0) || (m_source->currentIndex() > 0) || (m_language->currentIndex() > 0));
}

void Content::LibraryWidget::applyVariantFilters()
{
    QString pattern = u"^"_s;
    for (const QString &word : m_variantFilter->text().split(u' ', Qt::SkipEmptyParts))
        pattern += u"(?=.*%1)"_s.arg(QRegularExpression::escape(word));
    m_variantProxy->setFilterRegularExpression(QRegularExpression(pattern, QRegularExpression::CaseInsensitiveOption));
    m_variantProxy->setShowReported(m_showReported->isChecked());
    m_variantProxy->setFacets(m_variantFacets->isHidden() ? QJsonObject() : m_episode->currentData().toJsonObject(),
        m_variantFacets->isHidden() ? QVariant() : m_voice->currentData(),
        m_variantFacets->isHidden() ? QJsonObject() : m_source->currentData().toJsonObject(),
        m_variantFacets->isHidden() ? QVariant() : m_language->currentData());
}

void Content::LibraryWidget::filterVariants()
{
    const QJsonObject selected = currentVariant();
    const QSignalBlocker blocker(m_variants->selectionModel());
    applyVariantFilters();
    int selectedRow = -1;
    for (int row = 0; row < m_variantProxy->rowCount(); ++row)
    {
        const QJsonObject variant = m_variantProxy->index(row, VariantColumn).data(Qt::UserRole).toJsonObject();
        if (sameVariant(variant, selected))
            selectedRow = row;
    }
    if ((selectedRow < 0) && (m_variantProxy->rowCount() > 0))
        selectedRow = 0;
    if (selectedRow >= 0)
        m_variants->selectRow(selectedRow);
    else
    {
        m_variants->clearSelection();
        m_variants->setCurrentIndex({});
    }
    updateVariantColumns();
    selectVariant();
}

void Content::LibraryWidget::updateVariantColumns()
{
    bool hasSize = false;
    bool hasSeeds = false;
    for (int row = 0; row < m_variantProxy->rowCount(); ++row)
    {
        const QJsonObject variant = m_variantProxy->index(row, VariantColumn).data(Qt::UserRole).toJsonObject();
        hasSize |= variant.value(u"size"_s).toInteger() > 0;
        hasSeeds |= variant.value(u"seeders"_s).isDouble();
    }
    m_variants->setColumnHidden(SourceColumn, !m_variantFacets->isHidden() && (m_source->currentIndex() > 0));
    m_variants->setColumnHidden(SizeColumn, !hasSize);
    m_variants->setColumnHidden(SeedsColumn, !hasSeeds);
    m_variants->horizontalHeader()->setVisible(hasSize || hasSeeds);
}

void Content::LibraryWidget::showWork(const QJsonObject &work, const QJsonObject &preferredVariant, const bool revealPreferred,
    const QJsonValue &filters)
{
    const QJsonObject selectedVariant = preferredVariant.isEmpty() ? currentVariant() : preferredVariant;
    const QSignalBlocker selectionBlocker(m_variants->selectionModel());
    m_work = work;
    m_title->setText(workTitle(work));
    m_variantModel->removeRows(0, m_variantModel->rowCount());
    int selectedRow = -1;
    bool hasReported = false;
    bool selectedReported = false;
    QHash<QString, QSet<QString>> releaseTitles;
    const auto publicationVersion = [](const QJsonObject &variant)
    {
        return variant.value(u"sourceId"_s).toString() + u'\n' + variant.value(u"url"_s).toString()
            + u'\n' + variant.value(u"version"_s).toString();
    };
    QSet<QString> completeVersions;
    for (const QJsonValue &value : work.value(u"variants"_s).toArray())
    {
        const QJsonObject variant = value.toObject();
        if (!variant.value(u"needsInspection"_s).toBool()
            && (variant.value(u"transport"_s).toString() == u"torrent")
            && (variant.value(u"publicationKind"_s).toString() == u"full")
            && !variant.value(u"magnet"_s).toString().isEmpty()
            && !variant.value(u"version"_s).toString().isEmpty())
        {
            completeVersions.insert(publicationVersion(variant));
        }
    }
    for (const QJsonValue &value : work.value(u"variants"_s).toArray())
    {
        const QJsonObject variant = value.toObject();
        if (variant.value(u"needsInspection"_s).toBool()
            || ((variant.value(u"publicationKind"_s).toString() == u"archivePart")
                && completeVersions.contains(publicationVersion(variant)) && !sameVariant(variant, selectedVariant)))
        {
            continue;
        }
        QStringList labels;
        for (const QString &key : {u"version"_s, u"edition"_s, u"author"_s, u"translation"_s,
            u"player"_s, u"quality"_s})
        {
            const QString label = variant.value(key).toString();
            if (!label.isEmpty())
                labels << label;
        }
        for (const QJsonValue &value : variant.value(u"languages"_s).toArray())
            labels << languageName(value.toString());
        if (variant.value(u"cooperative"_s).toBool())
            labels.prepend(tr("Co-op"));
        if (!variant.value(u"episode"_s).toString().isEmpty() || !variant.value(u"season"_s).toString().isEmpty())
            labels << episodeLabel(variant);
        const QString publication = variant.value(u"publicationKind"_s).toString();
        if (publication == u"patch")
            labels.prepend(tr("Patch"));
        else if (publication == u"fix")
            labels.prepend(tr("Fix"));
        else if (publication == u"optional")
            labels.prepend(tr("Optional"));
        else if (publication == u"archivePart")
            labels.prepend(tr("Archive part"));
        if (labels.isEmpty())
            labels << variant.value(u"title"_s).toString();
        auto *release = new QStandardItem(labels.join(u", "_s));
        release->setData(variant, Qt::UserRole);
        release->setTextAlignment(Qt::AlignLeft | Qt::AlignVCenter);
        const QString transport = variant.value(u"transport"_s).toString();
        if (transport == u"torrent")
            release->setIcon(UIThemeManager::instance()->getIcon(u"torrent-magnet"_s));
        else if (transport == u"video")
            release->setIcon(UIThemeManager::instance()->getIcon(u"torrent-start"_s));
        else if ((transport == u"book") || (transport == u"manga"))
            release->setIcon(UIThemeManager::instance()->getIcon(u"help-contents"_s));
        else if (transport == u"file")
            release->setIcon(UIThemeManager::instance()->getIcon(u"fileicon"_s));
        release->setToolTip(release->text() + u'\n' + variant.value(u"title"_s).toString());
        auto *source = new QStandardItem(variant.value(u"sourceName"_s).toString());
        source->setToolTip(source->text());
        const QString filterText = source->text() + u' ' + labels.join(u' ') + u' '
            + variant.value(u"title"_s).toString();
        release->setData(filterText, Qt::UserRole + 1);
        releaseTitles[variant.value(u"sourceId"_s).toString() + u'\n' + release->text()]
            .insert(variant.value(u"title"_s).toString());
        const qint64 size = variant.value(u"size"_s).toInteger();
        const QJsonValue seeds = variant.value(u"seeders"_s);
        const bool reported = (transport == u"video")
            && (variant.value(u"adStatus"_s).toString() == u"reported");
        hasReported |= reported;
        if (sameVariant(variant, selectedVariant))
        {
            selectedRow = m_variantModel->rowCount();
            selectedReported = reported;
        }
        m_variantModel->appendRow({release, source,
            new QStandardItem(size > 0 ? Utils::Misc::friendlyUnit(size) : QString()),
            new QStandardItem(seeds.isDouble() ? QString::number(seeds.toInt()) : QString())});
    }
    for (int row = 0; row < m_variantModel->rowCount(); ++row)
    {
        QStandardItem *const release = m_variantModel->item(row, VariantColumn);
        const QJsonObject variant = release->data(Qt::UserRole).toJsonObject();
        const QString key = variant.value(u"sourceId"_s).toString() + u'\n' + release->text();
        const QString title = variant.value(u"title"_s).toString();
        if ((releaseTitles.value(key).size() > 1) && !title.isEmpty())
            release->setText(title + u" — "_s + release->text());
    }
    m_details->show();
    updateVariantFacets(work.value(u"variants"_s).toArray(), selectedVariant, revealPreferred, filters);
    applyVariantFilters();
    m_favorite->setChecked(libraryEntry(work).value(u"favorite"_s).toBool());
    m_variantFilter->setVisible((m_variantModel->rowCount() > 1) || !m_variantFilter->text().isEmpty());
    m_showReported->setVisible(hasReported);
    if (revealPreferred && selectedReported)
    {
        const QSignalBlocker blocker(m_showReported);
        m_showReported->setChecked(true);
        m_variantProxy->setShowReported(true);
    }
    const QModelIndex selected = selectedVariant.isEmpty() ? m_variantProxy->index(0, VariantColumn)
        : m_variantProxy->mapFromSource(m_variantModel->index(selectedRow, VariantColumn));
    if (selected.isValid())
        m_variants->selectRow(selected.row());
    else
    {
        m_variants->clearSelection();
        m_variants->setCurrentIndex({});
    }
    updateVariantColumns();
    selectVariant();
}

QJsonObject Content::LibraryWidget::currentVariant() const
{
    const QModelIndex index = m_variants->currentIndex();
    return index.isValid() ? index.siblingAtColumn(VariantColumn).data(Qt::UserRole).value<QJsonObject>() : QJsonObject();
}

void Content::LibraryWidget::selectVariant()
{
    const QJsonObject variant = currentVariant();
    QStringList notes;
    const QString description = m_work.value(u"description"_s).toString();
    if (!description.isEmpty())
        notes << description.toHtmlEscaped().replace(u'\n', u"<br>"_s);
    if (variant.value(u"adStatus"_s).toString() == u"reported")
        notes << tr("Advertising reported in this release").toHtmlEscaped();
    for (const QJsonArray &items : {m_work.value(u"notes"_s).toArray(), variant.value(u"notes"_s).toArray()})
    {
        for (const QJsonValue &value : items)
        {
            const QJsonObject note = value.toObject();
            QString text = note.value(u"text"_s).toString().toHtmlEscaped().replace(u'\n', u"<br>"_s);
            const QString author = note.value(u"author"_s).toString();
            if (!author.isEmpty())
                text.prepend(u"<b>%1</b><br>"_s.arg(author.toHtmlEscaped()));
            const QUrl url(note.value(u"url"_s).toString());
            if (!url.host().isEmpty() && ((url.scheme() == u"http") || (url.scheme() == u"https")))
                text += u" <a href=\"%1\">%2</a>"_s.arg(url.toString().toHtmlEscaped(), tr("Source"));
            notes << text;
        }
    }
    m_notes->setHtml(u"<p>%1</p>"_s.arg(notes.join(u"</p><p>"_s)));
    m_detailPages->setTabVisible(1, !notes.isEmpty());
    m_detailPages->tabBar()->setVisible(!notes.isEmpty());
    if (notes.isEmpty())
        m_detailPages->setCurrentIndex(0);
    updateActions();
}

void Content::LibraryWidget::updateActions()
{
    const QString transport = currentVariant().value(u"transport"_s).toString();
    const QString category = m_work.value(u"category"_s).toString();
    const bool torrentVideo = (transport == u"torrent")
        && ((category == u"movies") || (category == u"tv") || (category == u"anime"));
    const bool waitingForTorrent = !m_torrentPlayback.isEmpty();
    QString action = tr("Download");
    if (waitingForTorrent)
        action = tr("Cancel watch");
    else if ((transport == u"video") || torrentVideo)
        action = tr("Watch");
    else if ((transport == u"book") || (transport == u"manga"))
        action = tr("Read");
    m_open->setText(action);
    m_open->setEnabled(waitingForTorrent || (!transport.isEmpty() && !m_resolveRequest));
    m_favorite->setEnabled(!m_work.isEmpty() && !currentVariant().isEmpty());
    m_favorite->setIcon(UIThemeManager::instance()->getIcon(m_favorite->isChecked() ? u"bookmark-active"_s : u"bookmark"_s));
    const QString favoriteAction = m_favorite->isChecked() ? tr("Remove from favorites") : tr("Add to favorites");
    m_favorite->setToolTip(favoriteAction);
    m_favorite->setAccessibleName(favoriteAction);
    m_share->setEnabled(!m_work.isEmpty());
    m_download->setVisible((transport == u"video") || torrentVideo);
    m_download->setEnabled(!m_downloads->isPreparing() && !m_resolveRequest
        && (!waitingForTorrent || (transport != u"torrent")));
    if (transport == u"file")
        m_open->setEnabled(m_open->isEnabled() && !m_downloads->isPreparing());
    m_detailsProgress->setVisible(m_detailsRequest);
    const bool empty = m_variantProxy->rowCount() == 0;
    m_variants->setVisible(!empty);
    m_emptyVariants->setVisible(empty);
    m_emptyVariants->setText(m_detailsRequest ? tr("Loading versions…")
        : m_variantModel->rowCount() > 0 ? tr("No matching versions") : tr("No versions found"));
}

void Content::LibraryWidget::inspectWork()
{
    if (m_work.isEmpty() || m_detailsRequest)
        return;

    QJsonArray variants;
    bool pending = false;
    for (const QJsonValue &value : m_work.value(u"variants"_s).toArray())
    {
        const QJsonObject variant = value.toObject();
        if (variant.value(u"needsInspection"_s).toBool())
        {
            const QString identity = variant.value(u"sourceId"_s).toString() + u'\n' + variant.value(u"id"_s).toString();
            if (m_inspectedPublications.contains(identity))
                continue;

            m_inspectedPublications.insert(identity);
            pending = true;
        }
        variants.append(value);
    }
    if (!pending)
        return;

    QJsonObject work = m_work;
    work.insert(u"variants"_s, variants);
    m_detailsRequest = Client::instance()->request(u"inspectWork"_s, {{u"work"_s, work},
        {u"language"_s, QLocale(Preferences::instance()->getLocale()).bcp47Name()}});
    updateActions();
}

void Content::LibraryWidget::applyDetails(QJsonObject details)
{
    const QJsonObject publication = details.take(u"inspectedPublication"_s).toObject();
    const QJsonArray variants = m_work.value(u"variants"_s).toArray();
    if (publication.value(u"sourceId"_s).toString().isEmpty() || publication.value(u"url"_s).toString().isEmpty()
        || std::none_of(variants.cbegin(), variants.cend(), [&publication](const QJsonValue &value)
        {
            return samePublication(value.toObject(), publication);
        }))
    {
        return;
    }

    const QJsonObject selected = currentVariant();
    details.insert(u"id"_s, m_work.value(u"id"_s));
    const QJsonObject work = mergeWork(m_work, details, publication);
    const QModelIndex index = m_works->currentIndex().siblingAtColumn(0);
    m_works->model()->setData(index, work, Qt::UserRole);
    m_works->model()->setData(index, workTitle(work), Qt::DisplayRole);
    m_works->model()->setData(index, workToolTip(work), Qt::ToolTipRole);
    showWork(work, selected);
}

void Content::LibraryWidget::resolveVariant(const bool downloadOnly)
{
    if (currentVariant().isEmpty() || m_resolveRequest || !m_torrentPlayback.isEmpty())
        return;

    if (currentVariant().value(u"transport"_s).toString() == u"file")
    {
        downloadVariant();
        return;
    }

    m_resolvingWork = m_work;
    m_resolvingVariant = currentVariant();
    m_resolvingFilters = variantFilters();
    m_watchResolvedTorrent = !downloadOnly && (m_resolvingVariant.value(u"transport"_s).toString() == u"torrent");
    startResolve();
}

void Content::LibraryWidget::startResolve()
{
    m_openSource->hide();
    m_retry->hide();
    m_failureUrl.clear();
    m_resolveRequest = Client::instance()->request(u"resolve"_s,
        {{u"work"_s, m_resolvingWork}, {u"variant"_s, m_resolvingVariant}});
    m_status->setText(tr("Opening %1…").arg(m_resolvingVariant.value(u"sourceName"_s).toString()));
    updateActions();
}

void Content::LibraryWidget::downloadVariant()
{
    if (currentVariant().isEmpty())
        return;

    if (currentVariant().value(u"transport"_s).toString() == u"torrent")
    {
        resolveVariant(true);
        return;
    }
    m_downloads->enqueue({{u"work"_s, m_work}, {u"variant"_s, currentVariant()}});
}

Content::ReaderWidget *Content::LibraryWidget::ensureReader()
{
    if (!m_reader)
    {
        m_reader = new ReaderWidget(m_pages);
        m_pages->addWidget(m_reader);
        connect(m_reader, &ReaderWidget::closed, this, &LibraryWidget::changeView);
        connect(m_reader, &ReaderWidget::progressChanged, this, [](const QJsonObject &progress)
        {
            const QString id = progress.value(u"workId"_s).toString();
            if (!id.isEmpty())
            {
                QJsonObject position = progress;
                position.remove(u"documentId"_s);
                position.remove(u"workId"_s);
                position.insert(u"id"_s, id);
                Client::instance()->request(u"progress"_s, position);
            }
        });
    }
    return m_reader;
}

void Content::LibraryWidget::openResolved(const QJsonObject &resolved)
{
    const QString kind = resolved.value(u"kind"_s).toString();
    bool opened = false;
    if (kind == u"torrent")
    {
        const QByteArray data = QByteArray::fromBase64(resolved.value(u"torrentData"_s).toString().toLatin1());
        const QString source = data.isEmpty() ? resolved.value(u"url"_s).toString()
            : m_resolvingVariant.value(u"url"_s).toString();
        const auto descriptor = data.isEmpty() ? BitTorrent::TorrentDescriptor::parse(source)
            : BitTorrent::TorrentDescriptor::load(data);
        if (!descriptor)
        {
            m_status->setText(descriptor.error());
            return;
        }
        const QString category = m_resolvingWork.value(u"category"_s).toString();
        const bool watch = m_watchResolvedTorrent
            && ((category == u"movies") || (category == u"tv") || (category == u"anime"));
        if (watch)
        {
            m_torrentPlayback = {{u"work"_s, m_resolvingWork}, {u"variant"_s, m_resolvingVariant},
                {u"variantFilters"_s, m_resolvingFilters},
                {u"source"_s, source}};
            m_watchInfoHash = descriptor->infoHash();
            if (BitTorrent::Torrent *torrent = BitTorrent::Session::instance()->findTorrent(descriptor->infoHash()))
            {
                watchTorrent(torrent);
                return;
            }
        }
        opened = app()->addTorrentManager()->showTorrent(source, descriptor.value());
        if (!opened && watch)
        {
            clearTorrentWatch();
        }
        if (opened && watch)
        {
            m_status->setText(tr("Add the torrent to watch."));
            return;
        }
    }
    else if (kind == u"video")
    {
        Media::PlaybackOptions options;
        options.audioUrl = QUrl(resolved.value(u"audioUrl"_s).toString());
        options.proxyUrl = QUrl(resolved.value(u"proxyUrl"_s).toString());
        const QJsonObject audioTrack = resolved.value(u"audioTrack"_s).toObject();
        options.audioTrack.title = audioTrack.value(u"title"_s).toString();
        options.audioTrack.type = audioTrack.value(u"type"_s).toString();
        options.audioTrack.language = audioTrack.value(u"language"_s).toString();
        options.audioTrack.codec = audioTrack.value(u"codec"_s).toString();
        options.audioTrack.channels = audioTrack.value(u"channels"_s).toInt();
        const QString category = m_resolvingWork.value(u"category"_s).toString();
        if (category == u"anime")
            options.contentKind = u"anime"_s;
        else if ((category == u"movies") || (category == u"tv"))
            options.contentKind = u"movie"_s;
        options.identity = u"video:"_s + m_resolvingVariant.value(u"sourceId"_s).toString()
            + u':' + m_resolvingVariant.value(u"id"_s).toString();
        options.legacyIdentity = m_resolvingWork.value(u"id"_s).toString()
            + u"/" + m_resolvingVariant.value(u"id"_s).toString();
        const QJsonObject object = resolved.value(u"headers"_s).toObject();
        for (auto it = object.constBegin(); it != object.constEnd(); ++it)
            options.headers.insert(it.key(), it.value().toString());
        for (const QJsonValue &value : resolved.value(u"subtitles"_s).toArray())
            options.subtitles.append(QUrl(value.toObject().value(u"url"_s).toString()));
        opened = Media::Player::instance()->playUrl(QUrl(resolved.value(u"url"_s).toString()),
            m_resolvingWork.value(u"title"_s).toString(), options);
        if (opened)
        {
            m_playback = {{u"identity"_s, options.identity},
                {u"work"_s, m_resolvingWork}, {u"variant"_s, m_resolvingVariant}, {u"variantFilters"_s, m_resolvingFilters}};
        }
        return;
    }
    else if ((kind == u"book") || (kind == u"manga"))
    {
        ReaderWidget *reader = ensureReader();
        reader->open(m_resolvingWork, m_resolvingVariant, resolved);
        m_pages->setCurrentWidget(reader);
        opened = true;
    }
    if (opened)
    {
        m_rememberRequests.insert(Client::instance()->request(u"remember"_s,
            {{u"work"_s, m_resolvingWork}, {u"variant"_s, m_resolvingVariant},
                {u"variantFilters"_s, m_resolvingFilters}, {u"opened"_s, true}}));
        m_status->setText(tr("Opened %1").arg(m_resolvingWork.value(u"title"_s).toString()));
    }
}

void Content::LibraryWidget::watchTorrent(BitTorrent::Torrent *torrent)
{
    const QPointer<BitTorrent::Torrent> guard(torrent);
    if (m_watchTorrent != torrent)
    {
        if (m_watchTorrent)
            m_watchTorrent->releaseStreamingLease();
        m_watchTorrent = torrent;
        torrent->acquireStreamingLease();
    }
    if (torrent->isStopped())
        torrent->start();
    if (!torrent->hasMetadata())
    {
        m_status->setText(tr("Waiting for torrent metadata…"));
        updateActions();
        return;
    }
    if (torrent->isChecking())
    {
        m_status->setText(tr("Checking existing files before playback…"));
        updateActions();
        return;
    }
    const QJsonObject playback = std::exchange(m_torrentPlayback, {});
    const PathList paths = torrent->filePaths();
    QList<int> mediaFiles;
    QStringList choices;
    for (qsizetype index = 0; index < paths.size(); ++index)
    {
        if (Media::Player::isMediaFile(paths[index].toString()))
        {
            mediaFiles.append(index);
            choices.append(paths[index].toString());
        }
    }
    updateActions();
    if (mediaFiles.isEmpty())
    {
        clearTorrentWatch();
        m_status->setText(tr("This torrent has no playable media files."));
        return;
    }
    m_status->clear();
    if (mediaFiles.size() == 1)
    {
        playTorrentFile(torrent, mediaFiles.front(), playback);
        return;
    }

    auto *dialog = new QInputDialog(this);
    dialog->setAttribute(Qt::WA_DeleteOnClose);
    dialog->setWindowTitle(tr("Watch"));
    dialog->setLabelText(tr("File or episode"));
    dialog->setComboBoxItems(choices);
    dialog->setComboBoxEditable(false);
    connect(dialog, &QInputDialog::finished, this, [this, dialog, guard, choices, mediaFiles, playback](const int result)
    {
        if (!guard || (m_watchTorrent != guard))
            return;
        if (result != QDialog::Accepted)
        {
            clearTorrentWatch();
            m_status->setText(tr("Watch cancelled"));
            return;
        }
        playTorrentFile(guard, mediaFiles.at(choices.indexOf(dialog->textValue())), playback);
    });
    dialog->open();
}

void Content::LibraryWidget::playTorrentFile(BitTorrent::Torrent *torrent, const int fileIndex, const QJsonObject &playback)
{
    const QJsonObject work = playback.value(u"work"_s).toObject();
    const QString contentKind = (work.value(u"category"_s).toString() == u"anime") ? u"anime"_s : u"movie"_s;
    const QString identity = Media::Player::torrentIdentity(torrent, fileIndex);
    const bool opened = Media::Player::instance()->playTorrent(torrent, fileIndex, contentKind);
    clearTorrentWatch();
    if (!opened)
        return;
    m_playback = {{u"identity"_s, identity}, {u"work"_s, work}, {u"variant"_s, playback.value(u"variant"_s)},
        {u"variantFilters"_s, playback.value(u"variantFilters"_s)}};
}

void Content::LibraryWidget::clearTorrentWatch()
{
    if (m_watchTorrent)
        m_watchTorrent->releaseStreamingLease();
    m_watchTorrent = nullptr;
    m_torrentPlayback = {};
}

void Content::LibraryWidget::toggleFavorite()
{
    const QJsonObject params {{u"work"_s, m_work}, {u"variant"_s, currentVariant()}, {u"favorite"_s, m_favorite->isChecked()}};
    const qint64 id = Client::instance()->request(u"remember"_s, params);
    m_rememberRequests.insert(id);
    m_favorite->setEnabled(false);
}

void Content::LibraryWidget::shareWork()
{
    QJsonObject params {{u"work"_s, m_work}, {u"mode"_s, u"work"_s}};
    const QJsonObject variant = currentVariant();
    if (!variant.isEmpty())
        params.insert(u"variant"_s, variant);
    m_shareRequest = Client::instance()->request(u"share"_s, params);
}

void Content::LibraryWidget::showBrowserPrerequisite()
{
    if (!m_browserPrerequisite)
    {
        m_browserPrerequisite = new BrowserPrerequisiteWidget(m_pages);
        m_pages->addWidget(m_browserPrerequisite);
        connect(m_browserPrerequisite, &BrowserPrerequisiteWidget::backRequested, this,
            [this] { m_pages->setCurrentWidget(m_splitter); });
        connect(m_browserPrerequisite, &BrowserPrerequisiteWidget::connected, this, [this]
        {
            if (m_pages->currentWidget() == m_browserPrerequisite)
            {
                m_pages->setCurrentWidget(m_splitter);
                startResolve();
            }
        });
    }
    m_browserPrerequisite->setSource(m_resolvingVariant.value(u"sourceName"_s).toString());
    m_pages->setCurrentWidget(m_browserPrerequisite);
}

void Content::LibraryWidget::showSources(const QString &sourceId)
{
    auto *dialog = new SourcesDialog(this, sourceId);
    dialog->setAttribute(Qt::WA_DeleteOnClose);
    connect(dialog, &SourcesDialog::publicationInspected, this, &LibraryWidget::applyDetails);
    dialog->open();
}

void Content::LibraryWidget::handleEvent(const qint64 id, const QString &event, const QJsonValue &data)
{
    const bool terminal = (event == u"complete") || (event == u"error");
    const QString error = data.toObject().value(u"message"_s).toString();
    if (id == m_searchRequest)
    {
        if (event == u"removeResult")
        {
            const QString workId = data.toObject().value(u"id"_s).toString();
            const QString replacementId = data.toObject().value(u"replacementId"_s).toString();
            const int row = m_workRows.value(workId, -1);
            if (row >= 0)
            {
                const bool active = (m_views->currentIndex() == 0) && (m_work.value(u"id"_s).toString() == workId);
                if (active && m_workRows.contains(replacementId))
                {
                    const QSignalBlocker blocker(m_works->selectionModel());
                    const int replacementRow = m_workRows.value(replacementId);
                    const QJsonObject replacement = mergeWork(m_work,
                        m_searchModel->item(replacementRow, 0)->data(Qt::UserRole).toJsonObject());
                    m_searchModel->item(replacementRow, 0)->setData(replacement, Qt::UserRole);
                    m_searchModel->item(replacementRow, 0)->setText(workTitle(replacement));
                    m_searchModel->item(replacementRow, 0)->setToolTip(workToolTip(replacement));
                    m_works->selectRow(m_searchProxy->mapFromSource(m_searchModel->index(replacementRow, 0)).row());
                    showWork(replacement);
                    inspectWork();
                }
                m_workRows.remove(workId);
                for (auto it = m_workRows.begin(); it != m_workRows.end(); ++it)
                {
                    if (it.value() > row)
                        --it.value();
                }
                m_searchModel->removeRow(row);
            }
        }
        else if (event == u"result")
        {
            upsertWork(data.toObject());
        }
        if (terminal)
        {
            m_searchRequest = 0;
            m_search->setText(tr("Search"));
            if (m_views->currentIndex() == 0)
            {
                m_expand->setVisible((event != u"error") && !m_expandedSearch);
            }
            updateSearchView();
            emit searchFinished(event == u"error");
        }
        else if ((event == u"result") || (event == u"removeResult"))
        {
            updateSearchView();
        }
    }
    else if (id == m_libraryRequest)
    {
        if (event == u"library")
        {
            updateLibraryEntries(data.toArray());
            if (!m_work.isEmpty())
                m_favorite->setChecked(libraryEntry(m_work).value(u"favorite"_s).toBool());
        }
        else if (event == u"error")
        {
            m_status->setText(error);
        }
        if (terminal)
            m_libraryRequest = 0;
    }
    else if (id == m_detailsRequest)
    {
        if (event == u"details")
        {
            applyDetails(data.toObject());
        }
        if (terminal)
        {
            m_detailsRequest = 0;
            inspectWork();
            updateActions();
        }
    }
    else if (id == m_resolveRequest)
    {
        if (event == u"resolved")
            openResolved(data.toObject());
        if (terminal)
        {
            m_resolveRequest = 0;
            if (event == u"error")
            {
                const QJsonObject failure = data.toObject();
                if (failure.value(u"code"_s).toString() == u"browser_required")
                {
                    m_status->clear();
                    showBrowserPrerequisite();
                }
                else
                {
                    m_status->setText(error);
                    m_failureUrl = QUrl(failure.value(u"url"_s).toString());
                    const bool hasSourceUrl = m_failureUrl.isValid() && !m_failureUrl.host().isEmpty()
                        && ((m_failureUrl.scheme() == u"http") || (m_failureUrl.scheme() == u"https"));
                    m_openSource->setVisible(hasSourceUrl);
                    m_retry->setVisible(hasSourceUrl);
                }
            }
            updateActions();
        }
    }
    else if (id == m_linkRequest)
    {
        if (event == u"query")
        {
            const QJsonObject query = data.toObject();
            m_query->setText(query.value(u"query"_s).toString());
            const int category = m_category->findData(query.value(u"category"_s).toString());
            if (category >= 0)
                m_category->setCurrentIndex(category);
            search();
        }
        else if (event == u"details")
        {
            QJsonObject work = data.toObject();
            const QString captureAction = work.take(u"captureAction"_s).toString();
            m_query->setText(work.value(u"title"_s).toString());
            m_category->setCurrentIndex(m_category->findData(work.value(u"category"_s).toString()));
            upsertWork(work);
            const QSignalBlocker blocker(m_works->selectionModel());
            m_works->selectRow(m_searchProxy->mapFromSource(m_searchModel->index(m_workRows.value(work.value(u"id"_s).toString()), 0)).row());
            showWork(work, work.value(u"preferredVariant"_s).toObject(), true);
            updateSearchView();
            inspectWork();
            if (captureAction == u"watch")
                resolveVariant();
            else if (captureAction == u"download")
                downloadVariant();
            else
                m_status->setText(tr("Shared title [exact release]"));
        }
        else if (event == u"error")
        {
            m_status->setText(error);
        }
        if (terminal)
            m_linkRequest = 0;
    }
    else if (id == m_shareRequest)
    {
        if (event == u"share")
        {
            QGuiApplication::clipboard()->setText(data.toObject().value(u"url"_s).toString());
            m_status->setText(tr("Link copied"));
        }
        else if (event == u"error")
        {
            m_status->setText(error);
        }
    }
    else if (m_rememberRequests.contains(id) && terminal)
    {
        m_rememberRequests.remove(id);
        if (event == u"complete")
        {
            updateLibraryEntries({data});
            if (m_libraryRequest)
            {
                // A library request may still be streaming an older snapshot.
                Client::instance()->cancel(m_libraryRequest);
                m_libraryRequest = Client::instance()->request(u"library"_s);
            }
        }
        else
        {
            m_status->setText(error);
        }
        m_favorite->setChecked(libraryEntry(m_work).value(u"favorite"_s).toBool());
        updateActions();
    }
    else if ((id == m_removeRequest) && terminal)
    {
        if (event == u"complete")
        {
            m_details->hide();
            loadLibrary();
        }
        else
        {
            m_status->setText(error);
        }
    }
}
