use arrow::array::{Array, StringArray};
use arrow::record_batch::RecordBatch;

/// Zero-boilerplate helper for extracting typed values from Apache Arrow `RecordBatch` instances.
pub struct BatchAccessor<'a> {
    batch: &'a RecordBatch,
}

impl<'a> BatchAccessor<'a> {
    pub fn new(batch: &'a RecordBatch) -> Self {
        Self { batch }
    }

    /// Gets a `StringArray` reference for the given column name if present.
    pub fn str_col(&self, name: &str) -> Option<&'a StringArray> {
        let idx = self.batch.schema().index_of(name).ok()?;
        self.batch.column(idx).as_any().downcast_ref::<StringArray>()
    }

    /// Extract a `&str` value for `row` from optional column, defaulting to `""`.
    pub fn str_val(&self, col: Option<&'a StringArray>, row: usize) -> &'a str {
        col.and_then(|a| if a.is_valid(row) { Some(a.value(row)) } else { None })
            .unwrap_or("")
    }

    /// Extract an optional `String` for `row` if non-empty and valid.
    pub fn opt_str_val(&self, col: Option<&StringArray>, row: usize) -> Option<String> {
        let val = self.str_val(col, row);
        if val.is_empty() {
            None
        } else {
            Some(val.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::StringArray;
    use arrow::datatypes::{DataType, Field, Schema};
    use std::sync::Arc;

    #[test]
    fn test_batch_accessor() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("code", DataType::Utf8, false),
            Field::new("name", DataType::Utf8, true),
        ]));

        let code_arr = StringArray::from(vec!["A101", "A102"]);
        let name_arr = StringArray::from(vec![Some("Hospital A"), None]);

        let batch = RecordBatch::try_new(schema, vec![Arc::new(code_arr), Arc::new(name_arr)]).unwrap();
        let accessor = BatchAccessor::new(&batch);

        let code_col = accessor.str_col("code");
        let name_col = accessor.str_col("name");

        assert_eq!(accessor.str_val(code_col, 0), "A101");
        assert_eq!(accessor.str_val(name_col, 0), "Hospital A");
        assert_eq!(accessor.str_val(name_col, 1), "");

        assert_eq!(accessor.opt_str_val(name_col, 0), Some("Hospital A".to_string()));
        assert_eq!(accessor.opt_str_val(name_col, 1), None);
        assert_eq!(accessor.str_col("nonexistent"), None);
    }
}
