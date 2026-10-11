/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "chapterpicker.h"

#include <QItemSelectionModel>
#include <QLabel>
#include <QLineEdit>
#include <QListView>
#include <QScrollBar>
#include <QSortFilterProxyModel>
#include <QVBoxLayout>

#include "base/global.h"
#include "modaloverlay.h"

Content::ChapterPicker::ChapterPicker(QWidget *parent)
    : QComboBox(parent)
{
    connect(this, &QComboBox::currentTextChanged, this, &QWidget::setToolTip);
    connect(this, &QComboBox::currentIndexChanged, this, [this](const int index)
    {
        if (!m_overlay)
            return;
        m_list->selectionModel()->setCurrentIndex(m_filter->mapFromSource(model()->index(index, modelColumn())),
            QItemSelectionModel::ClearAndSelect);
    });
}

Content::ChapterPicker::~ChapterPicker()
{
    delete m_overlay.data();
}

void Content::ChapterPicker::resetPopup()
{
    hidePopup();
    delete m_overlay.data();
}

void Content::ChapterPicker::showPopup()
{
    if (count() == 0)
        return;

    const bool firstOpen = !m_overlay;
    if (firstOpen)
    {
        m_overlay = new ModalOverlay(window());
        m_overlay->setObjectName(u"readerChapterOverlay"_s);
        m_overlay->setTitle(tr("Chapters"));
        QWidget *content = m_overlay->contentWidget();
        auto *layout = new QVBoxLayout(content);
        layout->setContentsMargins(0, 0, 0, 0);
        layout->setSpacing(12);

        m_search = new QLineEdit(content);
        m_search->setObjectName(u"readerChapterSearch"_s);
        m_search->setPlaceholderText(tr("Search chapters"));
        m_search->setClearButtonEnabled(true);
        layout->addWidget(m_search);

        auto *empty = new QLabel(tr("No matching chapters"), content);
        empty->setAlignment(Qt::AlignCenter);
        layout->addWidget(empty);

        m_filter = new QSortFilterProxyModel(m_overlay);
        m_filter->setSourceModel(model());
        m_filter->setFilterKeyColumn(modelColumn());
        m_filter->setFilterCaseSensitivity(Qt::CaseInsensitive);

        m_list = new QListView(content);
        m_list->setObjectName(u"readerChapterList"_s);
        m_list->setModel(m_filter);
        m_list->setFrameShape(QFrame::NoFrame);
        m_list->setUniformItemSizes(true);
        m_list->setStyleSheet(u"QListView::item { padding: 10px 12px; }"_s);
        layout->addWidget(m_list, 1);

        const auto updateResults = [this, empty]
        {
            const bool hasMatches = m_filter->rowCount() > 0;
            empty->setVisible(!hasMatches);
            m_list->setVisible(hasMatches);
        };
        connect(m_search, &QLineEdit::textChanged, m_filter, &QSortFilterProxyModel::setFilterFixedString);
        connect(m_filter, &QAbstractItemModel::rowsInserted, m_overlay, updateResults);
        connect(m_filter, &QAbstractItemModel::rowsRemoved, m_overlay, updateResults);
        connect(m_filter, &QAbstractItemModel::modelReset, m_overlay, updateResults);
        updateResults();

        const auto activate = [this](const QModelIndex &index)
        {
            if (!m_overlay->isVisible() || !index.isValid())
                return;
            const int chapter = m_filter->mapToSource(index).row();
            m_overlay->close();
            setCurrentIndex(chapter);
            emit activated(chapter);
        };
        connect(m_list, &QListView::clicked, this, activate);
        connect(m_list, &QListView::activated, this, activate);
        connect(m_overlay, &ModalOverlay::closed, this, [this] { QComboBox::hidePopup(); });
        m_list->setCurrentIndex(m_filter->mapFromSource(model()->index(currentIndex(), modelColumn())));
    }

    const int scroll = m_list->verticalScrollBar()->value();
    m_overlay->open(m_search);
    if (firstOpen)
        m_list->scrollTo(m_list->currentIndex(), QAbstractItemView::PositionAtCenter);
    else
        m_list->verticalScrollBar()->setValue(scroll);
}

void Content::ChapterPicker::hidePopup()
{
    if (m_overlay && m_overlay->isVisible())
        m_overlay->close();
    else
        QComboBox::hidePopup();
}
