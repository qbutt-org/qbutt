// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QDialog>

class QComboBox;
class QJsonValue;
class QLabel;
class QLineEdit;
class QPushButton;

namespace Content
{
    class TranslationSettingsDialog final : public QDialog
    {
        Q_OBJECT

    public:
        explicit TranslationSettingsDialog(QWidget *parent = nullptr);
        ~TranslationSettingsDialog() override;

    private:
        void save();
        void handleEvent(qint64 id, const QString &event, const QJsonValue &data);

        QWidget *m_fields = nullptr;
        QComboBox *m_provider = nullptr;
        QLineEdit *m_apiKey = nullptr;
        QLineEdit *m_model = nullptr;
        QLineEdit *m_baseUrl = nullptr;
        QPushButton *m_save = nullptr;
        QLabel *m_status = nullptr;
        qint64 m_request = 0;
    };
}
