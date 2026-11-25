use std::{collections::HashMap, path::Path};

use p5099_rho5::{Rho5Directory, Rho5Limits};
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use thiserror::Error;

pub(crate) const SHOP_STOCK_PATH: &str = "zeta_/kr/shop/data/stock.kml";
const MAX_STOCK_SOURCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_STOCKS: usize = 200_000;
const MAX_ITEMS_PER_STOCK: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShopCurrency {
    Cash,
    Lucci,
    Other(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ShopStockItem {
    pub category: u16,
    pub id: u16,
    pub count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShopStock {
    pub price: u32,
    pub currency: ShopCurrency,
    pub on_sale: bool,
    pub items: Vec<ShopStockItem>,
}

#[derive(Debug, Default)]
pub(crate) struct ShopCatalog {
    stocks: HashMap<u32, ShopStock>,
}

#[derive(Debug, Error)]
pub(crate) enum ShopCatalogError {
    #[error("cannot scan the client Data directory: {0}")]
    Scan(String),
    #[error("{SHOP_STOCK_PATH} was not found in the client Data")]
    Missing,
    #[error("cannot extract {SHOP_STOCK_PATH}: {0}")]
    Extract(String),
    #[error("{SHOP_STOCK_PATH} is {0} bytes, over the bound")]
    TooLarge(usize),
    #[error("{SHOP_STOCK_PATH} is not valid UTF-16")]
    Encoding,
    #[error("{SHOP_STOCK_PATH} XML error: {0}")]
    Xml(String),
    #[error("{SHOP_STOCK_PATH} has more than {MAX_STOCKS} stocks")]
    TooManyStocks,
    #[error("{SHOP_STOCK_PATH} stock has an invalid {0}")]
    InvalidField(&'static str),
}

impl ShopCatalog {
    pub(crate) fn stock(&self, stock_id: u32) -> Option<&ShopStock> {
        self.stocks.get(&stock_id)
    }

    pub(crate) fn len(&self) -> usize {
        self.stocks.len()
    }

    #[cfg(test)]
    pub(crate) fn from_stocks(stocks: impl IntoIterator<Item = (u32, ShopStock)>) -> Self {
        Self {
            stocks: stocks.into_iter().collect(),
        }
    }
}

pub(crate) fn load_client_shop_catalog(
    data_directory: &Path,
) -> Result<ShopCatalog, ShopCatalogError> {
    let directory = Rho5Directory::scan_kr(data_directory, Rho5Limits::default())
        .map_err(|error| ShopCatalogError::Scan(error.to_string()))?;
    let entry = directory
        .entries()
        .iter()
        .rfind(|entry| {
            entry
                .normalized_path()
                .eq_ignore_ascii_case(SHOP_STOCK_PATH)
        })
        .ok_or(ShopCatalogError::Missing)?;
    if entry.plaintext_size() > MAX_STOCK_SOURCE_BYTES {
        return Err(ShopCatalogError::TooLarge(entry.plaintext_size()));
    }
    let bytes = directory
        .extract_entry_with_legacy_padding(entry)
        .map_err(|error| ShopCatalogError::Extract(error.to_string()))?;
    parse_stock_xml(&decode_text(&bytes)?)
}

fn decode_text(bytes: &[u8]) -> Result<String, ShopCatalogError> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.get(1) == Some(&0) {
        let body = &bytes[usize::from(bytes.starts_with(&[0xff, 0xfe])) * 2..];
        let units = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&units).map_err(|_| ShopCatalogError::Encoding)
    } else {
        String::from_utf8(bytes.to_vec()).map_err(|_| ShopCatalogError::Encoding)
    }
}

fn attribute(element: &BytesStart<'_>, name: &str) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|attribute| attribute.key.as_ref().eq_ignore_ascii_case(name.as_bytes()))
        .map(|attribute| String::from_utf8_lossy(&attribute.value).into_owned())
}

fn number<T: std::str::FromStr>(
    element: &BytesStart<'_>,
    name: &'static str,
) -> Result<T, ShopCatalogError> {
    attribute(element, name)
        .and_then(|value| value.trim().parse().ok())
        .ok_or(ShopCatalogError::InvalidField(name))
}

pub(crate) fn parse_stock_xml(xml: &str) -> Result<ShopCatalog, ShopCatalogError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut stocks = HashMap::new();
    let mut current: Option<(u32, ShopStock)> = None;
    loop {
        match reader
            .read_event()
            .map_err(|error| ShopCatalogError::Xml(error.to_string()))?
        {
            Event::Start(element) if element.local_name().as_ref() == b"stock" => {
                let id = number::<u32>(&element, "stockId")?;
                let price = number::<u32>(&element, "salePrice").ok();
                let currency = match number::<u8>(&element, "priceType").ok() {
                    Some(0) => ShopCurrency::Cash,
                    Some(1) => ShopCurrency::Lucci,
                    other => ShopCurrency::Other(other.unwrap_or(u8::MAX)),
                };
                current = Some((
                    id,
                    ShopStock {
                        price: price.unwrap_or(0),
                        currency,
                        on_sale: price.is_some()
                            && attribute(&element, "isOnSale")
                                .is_some_and(|value| value.eq_ignore_ascii_case("true")),
                        items: Vec::new(),
                    },
                ));
            }
            Event::Empty(element) | Event::Start(element)
                if element.local_name().as_ref() == b"item" =>
            {
                if let Some((_, stock)) = current.as_mut() {
                    if stock.items.len() >= MAX_ITEMS_PER_STOCK {
                        return Err(ShopCatalogError::InvalidField("item count"));
                    }
                    stock.items.push(ShopStockItem {
                        category: number(&element, "itemCatId")?,
                        id: number(&element, "itemId")?,
                        count: number(&element, "itemCount")?,
                    });
                }
            }
            Event::End(element) if element.local_name().as_ref() == b"stock" => {
                if let Some((id, stock)) = current.take() {
                    stocks.insert(id, stock);
                    if stocks.len() > MAX_STOCKS {
                        return Err(ShopCatalogError::TooManyStocks);
                    }
                }
            }
            Event::DocType(_) => return Err(ShopCatalogError::Xml("DOCTYPE".to_owned())),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(ShopCatalog { stocks })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stock_rows_with_their_items() {
        let catalog = parse_stock_xml(
            "<?xml version='1.0'?><stockList>\
             <stock stockId='33067' stockName='a' salePrice='1100' priceType='0' isOnSale='true'>\
             <item itemCatId='3' itemId='1426' itemCount='1' expireDay='7' /></stock>\
             <stock stockId='33069' stockName='b' salePrice='1800' priceType='1' isOnSale='false'>\
             <item itemCatId='23' itemId='108' itemCount='10' expireDay='0' />\
             <item itemCatId='23' itemId='109' itemCount='5' expireDay='0' /></stock>\
             <stock stockId='9' stockName='c' salePrice='5' priceType='3' isOnSale='true'></stock>\
             <stock stockId='220' stockName='coupon' modelName='coupon'></stock>\
             </stockList>",
        )
        .unwrap();
        assert_eq!(catalog.len(), 4);
        assert!(!catalog.stock(220).unwrap().on_sale);
        let kart = catalog.stock(33067).unwrap();
        assert_eq!(
            (kart.price, kart.currency, kart.on_sale),
            (1100, ShopCurrency::Cash, true)
        );
        assert_eq!(
            kart.items,
            [ShopStockItem {
                category: 3,
                id: 1426,
                count: 1
            }]
        );
        let pack = catalog.stock(33069).unwrap();
        assert_eq!(
            (pack.currency, pack.on_sale, pack.items.len()),
            (ShopCurrency::Lucci, false, 2)
        );
        assert_eq!(catalog.stock(9).unwrap().currency, ShopCurrency::Other(3));
        assert!(catalog.stock(1).is_none());
    }

    #[test]
    fn stock_client_price_list_loads_when_configured() {
        let Ok(data) = std::env::var("P5099_CLIENT_DATA_DIR") else {
            return;
        };
        let catalog = load_client_shop_catalog(Path::new(&data)).unwrap();
        assert!(catalog.len() > 30_000);
        let surfboard = catalog.stock(33_067).unwrap();
        assert_eq!(
            (surfboard.price, surfboard.currency),
            (1_100, ShopCurrency::Cash)
        );
        assert_eq!(
            (surfboard.items[0].category, surfboard.items[0].id),
            (3, 1_426)
        );
    }
}
