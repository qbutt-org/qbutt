// SPDX-License-Identifier: GPL-2.0-or-later
#include "add-torrent-layout.h"

#include <stdexcept>

#include <QComboBox>
#include <QCoreApplication>
#include <QDir>
#include <QHeaderView>
#include <QJsonArray>
#include <QJsonObject>
#include <QMap>
#include <QScopeGuard>
#include <QScrollBar>
#include <QStyle>
#include <QStyleOptionViewItem>
#include <QStyledItemDelegate>

#include "base/bittorrent/downloadpriority.h"
#include "base/global.h"
#include "base/settingsstorage.h"
#include "gui/addnewtorrentdialog.h"
#include "gui/torrentcontentmodel.h"
#include "gui/torrentcontentwidget.h"
#include "gui/uithememanager.h"

namespace
{
    class NameLayoutProbe final : public QStyledItemDelegate
    {
    public:
        NameLayoutProbe(QAbstractItemDelegate *delegate, TorrentContentWidget *tree)
            : QStyledItemDelegate(tree)
            , m_delegate {delegate}
            , m_tree {tree}
        {
        }

        void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override
        {
            if (!index.model()->hasChildren(index))
            {
                QStyleOptionViewItem styled {option};
                initStyleOption(&styled, index);
                const QRect textRect = styled.widget->style()->subElementRect(QStyle::SE_ItemViewItemText, &styled, styled.widget);
                files.insert(styled.text, QJsonObject {{u"name"_s, styled.text},
                    {u"textWidth"_s, textRect.width()},
                    {u"visibleWidth"_s, textRect.intersected(m_tree->viewport()->rect()).width()},
                    {u"readablePrefixWidth"_s, styled.fontMetrics.horizontalAdvance(styled.text.left(16))},
                    {u"requiredWidth"_s, styled.fontMetrics.horizontalAdvance(styled.text)},
                    {u"cellWidth"_s, styled.rect.width()}});
            }
            m_delegate->paint(painter, option, index);
        }

        mutable QMap<QString, QJsonObject> files;

    private:
        QAbstractItemDelegate *m_delegate;
        TorrentContentWidget *m_tree;
    };
}

void AddTorrentLayoutAcceptance::run(AddNewTorrentDialog *dialog, const QJsonObject &spec, QJsonObject &evidence)
{
    const QString headerKey = u"GUI/Qt6/AddNewTorrentDialog/TreeHeaderState"_s;
    if (SettingsStorage::instance()->hasKey(headerKey))
        throw std::runtime_error("Add Torrent layout requires a fresh profile without saved geometry");

    auto *tree = dialog->findChild<TorrentContentWidget *>(u"contentTreeView"_s);
    if (!tree)
        throw std::runtime_error("Native Add Torrent content layout is missing");
    if (tree->model()->headerData(TorrentContentWidget::Name, Qt::Horizontal).toString() != u"Имя"_s)
        throw std::runtime_error("Add Torrent layout requires the actual Russian translation");

    const ColorScheme originalScheme = UIThemeManager::instance()->colorScheme();
    QAbstractItemDelegate *columnDelegate = tree->itemDelegateForColumn(TorrentContentWidget::Name);
    NameLayoutProbe probe {columnDelegate ? columnDelegate : tree->itemDelegate(), tree};
    tree->setItemDelegateForColumn(TorrentContentWidget::Name, &probe);
    const auto restore = qScopeGuard([&]
    {
        tree->setItemDelegateForColumn(TorrentContentWidget::Name, columnDelegate);
        UIThemeManager::instance()->previewColorScheme(originalScheme);
        QCoreApplication::processEvents();
    });
    const QDir screenshots {spec.value(u"screenshots"_s).toString()};
    QJsonArray renders;
    QStringList failures;
    for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark})
    {
        UIThemeManager::instance()->previewColorScheme(scheme);
        const QString theme = (scheme == ColorScheme::Light) ? u"light"_s : u"dark"_s;
        for (const QSize size : {QSize {900, 680}, QSize {640, 480}, QSize {1100, 780}})
        {
            dialog->resize(size);
            tree->expandAll();
            tree->horizontalScrollBar()->triggerAction(QAbstractSlider::SliderToMinimum);
            QCoreApplication::processEvents();
            QCoreApplication::processEvents();
            const QString screenshot = screenshots.filePath(u"add-torrent-fresh-ru-%1-%2.png"_s.arg(theme).arg(size.width()));
            probe.files.clear();
            if (!dialog->grab().save(screenshot))
                throw std::runtime_error("Cannot capture the native Add Torrent layout");

            QJsonArray files;
            for (const QJsonObject &file : probe.files)
            {
                const int visibleWidth = file.value(u"visibleWidth"_s).toInt();
                const int prefixWidth = file.value(u"readablePrefixWidth"_s).toInt();
                if (visibleWidth < prefixWidth)
                    failures.append(u"%1/%2/%3: %4 px visible, %5 px needed for readable filename"_s
                        .arg(theme).arg(size.width()).arg(file.value(u"name"_s).toString()).arg(visibleWidth).arg(prefixWidth));
                files.append(file);
            }
            renders.append(QJsonObject {{u"theme"_s, theme}, {u"screenshot"_s, screenshot},
                {u"width"_s, dialog->width()}, {u"height"_s, dialog->height()},
                {u"nameWidth"_s, tree->columnWidth(TorrentContentWidget::Name)},
                {u"files"_s, files}});
        }
    }
    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    checks.append(QJsonObject {{u"name"_s, u"add-torrent-fresh-layout"_s}, {u"freshGeometry"_s, true},
        {u"renders"_s, renders}, {u"failures"_s, QJsonArray::fromStringList(failures)}});
    evidence.insert(u"checks"_s, checks);
    for (const QJsonValue &render : renders)
    {
        if (render.toObject().value(u"files"_s).toArray().size() != 3)
            throw std::runtime_error("Generated Add Torrent layout did not expose all three legal fixture files");
    }
    if (!failures.isEmpty())
        throw std::runtime_error(u"Fresh Add Torrent filenames are clipped: %1"_s.arg(failures.join(u"; "_s)).toStdString());

    dialog->resize(900, 680);
    QCoreApplication::processEvents();
    tree->horizontalScrollBar()->triggerAction(QAbstractSlider::SliderToMaximum);
    QCoreApplication::processEvents();
    const QModelIndex root = tree->model()->index(0, TorrentContentWidget::Name);
    const QModelIndex priority = tree->model()->index(tree->model()->rowCount(root) - 1, TorrentContentWidget::Priority, root);
    if (!tree->viewport()->visibleRegion().contains(tree->visualRect(priority)))
    {
        throw std::runtime_error("Horizontal scrolling did not reveal the native priority column");
    }
    tree->edit(priority);
    QCoreApplication::processEvents();
    auto *editor = tree->findChild<QComboBox *>();
    if (!editor || !editor->isVisible())
        throw std::runtime_error("Scrolled priority column does not open its native editor");
    editor->setCurrentIndex(2);
    if (priority.data(TorrentContentModel::UnderlyingDataRole).toInt() != static_cast<int>(BitTorrent::DownloadPriority::High))
        throw std::runtime_error("Native priority editor did not update the torrent selection");
    const QString priorityScreenshot = screenshots.filePath(u"add-torrent-fresh-ru-dark-900-priority.png"_s);
    if (!dialog->grab().save(priorityScreenshot))
        throw std::runtime_error("Cannot capture the visible native priority editor");
    evidence.insert(u"addTorrentPriorityScreenshot"_s, priorityScreenshot);
    editor->setCurrentIndex(1);
    tree->closePersistentEditor(priority);

    dialog->resize(1100, 780);
    tree->header()->setSectionResizeMode(TorrentContentWidget::Name, QHeaderView::Interactive);
    tree->setColumnWidth(TorrentContentWidget::Name, 280);
    QCoreApplication::processEvents();
    evidence.insert(u"addTorrentUserLayout"_s, QJsonObject {
        {u"header"_s, QString::fromLatin1(tree->header()->saveState().toBase64())}});
}

void AddTorrentLayoutAcceptance::verifyRestored(AddNewTorrentDialog *dialog, QJsonObject &evidence)
{
    auto *tree = dialog->findChild<TorrentContentWidget *>(u"contentTreeView"_s);
    const QJsonObject saved = evidence.value(u"addTorrentUserLayout"_s).toObject();
    if (!tree || (QString::fromLatin1(tree->header()->saveState().toBase64()) != saved.value(u"header"_s).toString()))
        throw std::runtime_error("Add Torrent replaced the saved file column layout");
    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    checks.append(QJsonObject {{u"name"_s, u"add-torrent-user-layout-restored"_s},
        {u"horizontalScrollPriorityEditor"_s, true}});
    evidence.insert(u"checks"_s, checks);
}
