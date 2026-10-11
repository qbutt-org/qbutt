/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "readersettingsdialog.h"

#include <QCheckBox>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QFormLayout>
#include <QScrollArea>
#include <QSignalBlocker>
#include <QSpinBox>
#include <QVBoxLayout>

#include "base/global.h"
#include "base/preferences.h"

Content::ReaderSettingsDialog::ReaderSettingsDialog(QWidget *parent)
    : QDialog(parent)
{
    setObjectName(u"readerSettingsDialog"_s);
    setWindowTitle(tr("Reader settings"));
    setMinimumSize(320, 180);
    resize(420, 310);

    auto *layout = new QVBoxLayout(this);
    auto *scroll = new QScrollArea(this);
    scroll->setWidgetResizable(true);
    scroll->setFrameShape(QFrame::NoFrame);
    auto *page = new QWidget(scroll);
    auto *form = new QFormLayout(page);
    form->setContentsMargins(0, 0, 0, 0);
    form->setFormAlignment(Qt::AlignTop);
    form->setFieldGrowthPolicy(QFormLayout::AllNonFixedFieldsGrow);
    form->setRowWrapPolicy(QFormLayout::WrapLongRows);

    m_mode = new QComboBox(page);
    m_mode->setObjectName(u"readerMode"_s);
    m_mode->addItem(tr("Page by page"), static_cast<int>(ReaderMode::Paged));
    m_mode->addItem(tr("Continuous vertical"), static_cast<int>(ReaderMode::Continuous));
    form->addRow(tr("Reading mode:"), m_mode);

    m_imageFit = new QComboBox(page);
    m_imageFit->setObjectName(u"readerImageFit"_s);
    m_imageFit->addItem(tr("Screen height"), static_cast<int>(ImageFit::Height));
    m_imageFit->addItem(tr("Width"), static_cast<int>(ImageFit::Width));
    form->addRow(tr("Fit image:"), m_imageFit);

    m_enlargeSmallImages = new QCheckBox(tr("Enlarge small images"), page);
    m_enlargeSmallImages->setObjectName(u"readerEnlargeSmallImages"_s);
    form->addRow(m_enlargeSmallImages);

    m_brightness = new QSpinBox(page);
    m_brightness->setObjectName(u"readerBrightness"_s);
    m_brightness->setRange(30, 100);
    m_brightness->setSuffix(tr("%"));
    form->addRow(tr("Brightness:"), m_brightness);

    m_pageGap = new QSpinBox(page);
    m_pageGap->setObjectName(u"readerPageGap"_s);
    m_pageGap->setRange(0, 48);
    m_pageGap->setSuffix(tr(" px"));
    form->addRow(tr("Page gap:"), m_pageGap);

    m_contentWidth = new QSpinBox(page);
    m_contentWidth->setObjectName(u"readerContentWidth"_s);
    m_contentWidth->setRange(50, 100);
    m_contentWidth->setSuffix(tr("%"));
    form->addRow(tr("Content width:"), m_contentWidth);

    scroll->setWidget(page);
    layout->addWidget(scroll, 1);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Close, this);
    layout->addWidget(buttons);
    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);

    loadSettings();
    connect(m_mode, &QComboBox::currentIndexChanged, this, &ReaderSettingsDialog::saveSettings);
    connect(m_imageFit, &QComboBox::currentIndexChanged, this, &ReaderSettingsDialog::saveSettings);
    connect(m_enlargeSmallImages, &QCheckBox::toggled, this, &ReaderSettingsDialog::saveSettings);
    connect(m_brightness, &QSpinBox::valueChanged, this, &ReaderSettingsDialog::saveSettings);
    connect(m_pageGap, &QSpinBox::valueChanged, this, &ReaderSettingsDialog::saveSettings);
    connect(m_contentWidth, &QSpinBox::valueChanged, this, &ReaderSettingsDialog::saveSettings);
    connect(Preferences::instance(), &Preferences::readerSettingsChanged, this, &ReaderSettingsDialog::loadSettings);
}

void Content::ReaderSettingsDialog::loadSettings()
{
    const ReaderSettings settings = Preferences::instance()->readerSettings();
    const QSignalBlocker modeBlocker(m_mode);
    const QSignalBlocker imageFitBlocker(m_imageFit);
    const QSignalBlocker enlargeBlocker(m_enlargeSmallImages);
    const QSignalBlocker brightnessBlocker(m_brightness);
    const QSignalBlocker pageGapBlocker(m_pageGap);
    const QSignalBlocker contentWidthBlocker(m_contentWidth);

    m_mode->setCurrentIndex(m_mode->findData(static_cast<int>(settings.mode)));
    m_imageFit->setCurrentIndex(m_imageFit->findData(static_cast<int>(settings.imageFit)));
    m_enlargeSmallImages->setChecked(settings.enlargeSmallImages);
    m_brightness->setValue(settings.brightness);
    m_pageGap->setValue(settings.pageGap);
    m_pageGap->setEnabled(settings.mode == ReaderMode::Continuous);
    m_contentWidth->setValue(settings.contentWidth);
}

void Content::ReaderSettingsDialog::saveSettings()
{
    ReaderSettings settings;
    settings.mode = static_cast<ReaderMode>(m_mode->currentData().toInt());
    settings.imageFit = static_cast<ImageFit>(m_imageFit->currentData().toInt());
    settings.enlargeSmallImages = m_enlargeSmallImages->isChecked();
    settings.brightness = m_brightness->value();
    settings.pageGap = m_pageGap->value();
    settings.contentWidth = m_contentWidth->value();
    Preferences::instance()->setReaderSettings(settings);
}
