// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QList>
#include <QMetaObject>
#include <QSortFilterProxyModel>
#include <QStringList>
#include <QTimer>
#include <QVariantList>
#include <QtQmlIntegration/qqmlintegration.h>

namespace Content
{
    class VariantFilterModel : public QSortFilterProxyModel
    {
        Q_OBJECT
        QML_ELEMENT

        Q_PROPERTY(QString episodeFilter MEMBER m_episodeFilter NOTIFY filtersChanged)
        Q_PROPERTY(QString translationFilter MEMBER m_translationFilter NOTIFY filtersChanged)
        Q_PROPERTY(QString sourceFilter MEMBER m_sourceFilter NOTIFY filtersChanged)
        Q_PROPERTY(QString playerFilter MEMBER m_playerFilter NOTIFY filtersChanged)
        Q_PROPERTY(QString languageFilter MEMBER m_languageFilter NOTIFY filtersChanged)
        Q_PROPERTY(QString transportFilter MEMBER m_transportFilter NOTIFY filtersChanged)
        Q_PROPERTY(QString query MEMBER m_query NOTIFY filtersChanged)
        Q_PROPERTY(QVariantList episodeOptions READ episodeOptions NOTIFY optionsChanged)
        Q_PROPERTY(QVariantList translationOptions READ translationOptions NOTIFY optionsChanged)
        Q_PROPERTY(QVariantList providerOptions READ providerOptions NOTIFY optionsChanged)
        Q_PROPERTY(QVariantList languageOptions READ languageOptions NOTIFY optionsChanged)
        Q_PROPERTY(bool hasTorrents READ hasTorrents NOTIFY optionsChanged)
        Q_PROPERTY(QString selectedId MEMBER m_selectedId NOTIFY selectedIdChanged)
        Q_PROPERTY(int selectedRow READ selectedRow NOTIFY selectedRowChanged)

    public:
        explicit VariantFilterModel(QObject *parent = nullptr);

        void setSourceModel(QAbstractItemModel *source) override;

        QVariantList episodeOptions() const;
        QVariantList translationOptions() const;
        QVariantList providerOptions() const;
        QVariantList languageOptions() const;
        bool hasTorrents() const;
        int selectedRow() const;

        Q_INVOKABLE QString idAt(int row) const;
        Q_INVOKABLE int indexOfId(const QString &id) const;

    signals:
        void filtersChanged();
        void optionsChanged();
        void selectedIdChanged();
        void selectedRowChanged();

    protected:
        bool filterAcceptsRow(int sourceRow, const QModelIndex &sourceParent) const override;

    private:
        void scheduleOptionsUpdate();
        void rebuildOptions();

        QString m_episodeFilter;
        QString m_translationFilter;
        QString m_sourceFilter;
        QString m_playerFilter;
        QString m_languageFilter;
        QString m_transportFilter;
        QString m_query;
        QString m_selectedId;
        QStringList m_queryTokens;

        int m_idRole = -1;
        int m_labelRole = -1;
        int m_episodeRole = -1;
        int m_translationRole = -1;
        int m_sourceIdRole = -1;
        int m_sourceRole = -1;
        int m_playerRole = -1;
        int m_languagesRole = -1;
        int m_kindRole = -1;
        int m_qualityRole = -1;

        QVariantList m_episodeOptions;
        QVariantList m_translationOptions;
        QVariantList m_providerOptions;
        QVariantList m_languageOptions;
        bool m_hasTorrents = false;
        QTimer m_optionsTimer;
        QList<QMetaObject::Connection> m_sourceConnections;
    };
}
