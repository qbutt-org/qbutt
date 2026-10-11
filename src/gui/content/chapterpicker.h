/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QComboBox>
#include <QPointer>

class QLineEdit;
class QListView;
class QSortFilterProxyModel;

namespace Content
{
    class ModalOverlay;

    class ChapterPicker final : public QComboBox
    {
        Q_OBJECT
        Q_DISABLE_COPY_MOVE(ChapterPicker)

    public:
        explicit ChapterPicker(QWidget *parent = nullptr);
        ~ChapterPicker() override;

        void resetPopup();
        void showPopup() override;
        void hidePopup() override;

    private:
        QPointer<ModalOverlay> m_overlay;
        QLineEdit *m_search = nullptr;
        QListView *m_list = nullptr;
        QSortFilterProxyModel *m_filter = nullptr;
    };
}
