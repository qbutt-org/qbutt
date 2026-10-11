#include "qbutt_quick/native.h"

#include <QCoreApplication>
#include <QLocale>
#include <QLoggingCategory>

#include "qbutt_quick/src/models.cxxqt.h"

Q_LOGGING_CATEGORY(contentLog, "qbutt.content")

std::unique_ptr<Qbutt::WorkModel> Qbutt::createWorkModel()
{
    return std::make_unique<WorkModel>();
}

std::unique_ptr<Qbutt::VariantModel> Qbutt::createVariantModel()
{
    return std::make_unique<VariantModel>();
}

QString Qbutt::contentError(const std::int32_t operation)
{
    switch (operation)
    {
    case 0:
        return QCoreApplication::translate("ContentController", "Could not start content services.");
    case 1:
        return QCoreApplication::translate("ContentController", "Search could not be completed.");
    case 2:
        return QCoreApplication::translate("ContentController", "Could not load variants.");
    case 3:
        return QCoreApplication::translate("ContentController", "Could not start the download.");
    case 4:
        return QCoreApplication::translate("ContentController", "Could not open the content library.");
    default:
        return QCoreApplication::translate("ContentController", "Could not save the content library.");
    }
}

QString Qbutt::formatSize(const std::uint64_t bytes)
{
    return QLocale().formattedDataSize(static_cast<qint64>(bytes));
}

void Qbutt::logContentError(const QString &operation, const QString &detail)
{
    qCWarning(contentLog).noquote() << operation << ':' << detail;
}
