/*
 * Bittorrent Client using Qt and libtorrent.
 * Copyright (C) 2006  Christophe Dumez <chris@qbittorrent.org>
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

#include "speedlimitdialog.h"

#include <algorithm>

#include <QStyle>

#include "base/bittorrent/session.h"
#include "ui_speedlimitdialog.h"
#include "uithememanager.h"
#include "utils.h"

#define SETTINGS_KEY(name) u"SpeedLimitDialog/" name

namespace
{
    void updateSliderValue(QSlider *slider, const int value)
    {
        if (value > slider->maximum())
            slider->setMaximum(value);
        slider->setValue(value);
    }
}

SpeedLimitDialog::SpeedLimitDialog(QWidget *parent)
    : QDialog {parent}
    , m_ui {new Ui::SpeedLimitDialog}
    , m_storeDialogSize {SETTINGS_KEY(u"Size"_s)}
{
    m_ui->setupUi(this);

    connect(m_ui->buttonBox, &QDialogButtonBox::accepted, this, &QDialog::accept);
    connect(m_ui->buttonBox, &QDialogButtonBox::rejected, this, &QDialog::reject);

    m_ui->labelSpeedIcon->setPixmap(
            UIThemeManager::instance()->getScaledPixmap(u"slow"_s, Utils::Gui::mediumIconSize(this).height()));

    const auto *session = BitTorrent::Session::instance();
    m_ui->spinUploadLimit->setValue(session->configuredUploadSpeedLimit() / 125000.);
    m_ui->spinDownloadLimit->setValue(session->configuredDownloadSpeedLimit() / 125000.);
    m_ui->sliderUploadLimit->setMaximum(10000);
    m_ui->sliderDownloadLimit->setMaximum(10000);
    updateSliderValue(m_ui->sliderUploadLimit, qRound(m_ui->spinUploadLimit->value() * 100));
    updateSliderValue(m_ui->sliderDownloadLimit, qRound(m_ui->spinDownloadLimit->value() * 100));

    m_initialValues =
    {
        m_ui->spinUploadLimit->value(),
        m_ui->spinDownloadLimit->value()
    };

    // Sync up/down speed limit sliders with their corresponding spinboxes
    connect(m_ui->sliderUploadLimit, &QSlider::valueChanged
            , m_ui->spinUploadLimit, [this](const int value) { m_ui->spinUploadLimit->setValue(value / 100.); });
    connect(m_ui->sliderDownloadLimit, &QSlider::valueChanged
            , m_ui->spinDownloadLimit, [this](const int value) { m_ui->spinDownloadLimit->setValue(value / 100.); });
    connect(m_ui->spinUploadLimit, qOverload<double>(&QDoubleSpinBox::valueChanged)
            , this, [this](const double value) { updateSliderValue(m_ui->sliderUploadLimit, qRound(value * 100)); });
    connect(m_ui->spinDownloadLimit, qOverload<double>(&QDoubleSpinBox::valueChanged)
            , this, [this](const double value) { updateSliderValue(m_ui->sliderDownloadLimit, qRound(value * 100)); });

    if (const QSize dialogSize = m_storeDialogSize; dialogSize.isValid())
        resize(dialogSize);
}

SpeedLimitDialog::~SpeedLimitDialog()
{
    m_storeDialogSize = size();
    delete m_ui;
}

void SpeedLimitDialog::accept()
{
    auto *session = BitTorrent::Session::instance();
    if (m_initialValues.uploadSpeedLimit != m_ui->spinUploadLimit->value())
        session->setConfiguredUploadSpeedLimit(qRound(m_ui->spinUploadLimit->value() * 125000));

    if (m_initialValues.downloadSpeedLimit != m_ui->spinDownloadLimit->value())
        session->setConfiguredDownloadSpeedLimit(qRound(m_ui->spinDownloadLimit->value() * 125000));

    QDialog::accept();
}
