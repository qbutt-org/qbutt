/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QDialog>

class QCheckBox;
class QComboBox;
class QSpinBox;

namespace Content
{
    class ReaderSettingsDialog final : public QDialog
    {
        Q_OBJECT
        Q_DISABLE_COPY_MOVE(ReaderSettingsDialog)

    public:
        explicit ReaderSettingsDialog(QWidget *parent = nullptr);

    private:
        void loadSettings();
        void saveSettings();

        QComboBox *m_mode = nullptr;
        QComboBox *m_imageFit = nullptr;
        QCheckBox *m_enlargeSmallImages = nullptr;
        QSpinBox *m_brightness = nullptr;
        QSpinBox *m_pageGap = nullptr;
        QSpinBox *m_contentWidth = nullptr;
    };
}
